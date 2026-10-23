//! Minimal S3-compatible signed object reads.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::SystemTime;

use aws_credential_types::Credentials;
use aws_sigv4::http_request::{
    sign, PayloadChecksumKind, PercentEncodingMode, SignableBody, SignableRequest, SigningSettings,
    UriPathNormalizationMode,
};
use aws_sigv4::sign::v4;
use reqwest::{Client, Url};

pub type StoreFuture<'a> = Pin<Box<dyn Future<Output = Result<Vec<u8>, StoreError>> + Send + 'a>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Missing,
    Unavailable,
}

pub trait ObjectStore: Send + Sync {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a>;
}

struct MissingObjectStore;

impl ObjectStore for MissingObjectStore {
    fn get<'a>(&'a self, _key: &'a str) -> StoreFuture<'a> {
        Box::pin(async { Err(StoreError::Missing) })
    }
}

struct FailedObjectStore;

impl ObjectStore for FailedObjectStore {
    fn get<'a>(&'a self, _key: &'a str) -> StoreFuture<'a> {
        Box::pin(async { Err(StoreError::Unavailable) })
    }
}

struct S3Environment {
    bucket: String,
    region: String,
    endpoint: String,
    access_key_id: String,
    secret_access_key: String,
    session_token: Option<String>,
    use_path_style: bool,
}

fn read_s3_environment(
    getenv: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> S3Environment {
    let read = |name: &str| getenv(name).unwrap_or_default();
    let region = match read("AWS_REGION") {
        value if !value.is_empty() => value,
        _ => match read("AWS_DEFAULT_REGION") {
            value if !value.is_empty() => value,
            _ => "us-east-1".to_owned(),
        },
    };
    let endpoint = match read("AWS_ENDPOINT_URL") {
        value if !value.is_empty() => value,
        _ => read("AWS_ENDPOINT"),
    };
    let session_token = match read("AWS_SESSION_TOKEN") {
        value if value.is_empty() => None,
        value => Some(value),
    };

    S3Environment {
        bucket: read("AWS_BUCKET"),
        region,
        endpoint,
        access_key_id: read("AWS_ACCESS_KEY_ID"),
        secret_access_key: read("AWS_SECRET_ACCESS_KEY"),
        session_token,
        use_path_style: matches!(
            read("AWS_USE_PATH_STYLE_ENDPOINT").as_str(),
            "1" | "t" | "T" | "true" | "TRUE" | "True"
        ),
    }
}

pub fn object_store_from_env(
    getenv: impl Fn(&str) -> Result<String, std::env::VarError>,
) -> Arc<dyn ObjectStore> {
    let environment = read_s3_environment(getenv);
    if environment.bucket.is_empty() {
        return Arc::new(MissingObjectStore);
    }
    if environment.access_key_id.is_empty() || environment.secret_access_key.is_empty() {
        return Arc::new(FailedObjectStore);
    }

    Arc::new(S3ObjectStore {
        client: Client::new(),
        environment,
        signing_time: Arc::new(SystemTime::now),
    })
}

struct S3ObjectStore {
    client: Client,
    environment: S3Environment,
    signing_time: Arc<dyn Fn() -> SystemTime + Send + Sync>,
}

impl S3ObjectStore {
    fn object_url(&self, key: &str) -> Result<Url, StoreError> {
        let endpoint = if self.environment.endpoint.is_empty() {
            format!("https://s3.{}.amazonaws.com", self.environment.region)
        } else {
            self.environment.endpoint.clone()
        };
        let mut url = Url::parse(&endpoint).map_err(|_| StoreError::Unavailable)?;
        if self.environment.use_path_style {
            append_path(&mut url, &self.environment.bucket, key)?;
        } else {
            let host = url.host_str().ok_or(StoreError::Unavailable)?;
            url.set_host(Some(&format!("{}.{}", self.environment.bucket, host)))
                .map_err(|_| StoreError::Unavailable)?;
            append_path(&mut url, "", key)?;
        }
        Ok(url)
    }

    fn signed_request(&self, key: &str) -> Result<reqwest::Request, StoreError> {
        let mut request = self
            .client
            .get(self.object_url(key)?)
            .build()
            .map_err(|_| StoreError::Unavailable)?;
        let identity = Credentials::new(
            self.environment.access_key_id.clone(),
            self.environment.secret_access_key.clone(),
            self.environment.session_token.clone(),
            None,
            "cloud-environment",
        )
        .into();
        let mut settings = SigningSettings::default();
        settings.payload_checksum_kind = PayloadChecksumKind::XAmzSha256;
        settings.percent_encoding_mode = PercentEncodingMode::Single;
        settings.uri_path_normalization_mode = UriPathNormalizationMode::Disabled;
        let parameters: aws_sigv4::http_request::SigningParams<'_> = v4::SigningParams::builder()
            .identity(&identity)
            .region(&self.environment.region)
            .name("s3")
            .time((self.signing_time)())
            .settings(settings)
            .build()
            .map_err(|_| StoreError::Unavailable)?
            .into();
        let headers: Vec<_> = request
            .headers()
            .iter()
            .filter_map(|(name, value)| value.to_str().ok().map(|value| (name.as_str(), value)))
            .collect();
        let signable = SignableRequest::new(
            request.method().as_str(),
            request.url().as_str(),
            headers.into_iter(),
            SignableBody::Bytes(&[]),
        )
        .map_err(|_| StoreError::Unavailable)?;
        let (instructions, _) = sign(signable, &parameters)
            .map_err(|_| StoreError::Unavailable)?
            .into_parts();
        for (name, value) in instructions.headers() {
            let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                .map_err(|_| StoreError::Unavailable)?;
            let value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| StoreError::Unavailable)?;
            request.headers_mut().insert(name, value);
        }
        Ok(request)
    }
}

impl ObjectStore for S3ObjectStore {
    fn get<'a>(&'a self, key: &'a str) -> StoreFuture<'a> {
        Box::pin(async move {
            let request = self.signed_request(key)?;
            let response = self
                .client
                .execute(request)
                .await
                .map_err(|_| StoreError::Unavailable)?;
            if response.status() == reqwest::StatusCode::NOT_FOUND {
                return Err(StoreError::Missing);
            }
            if !response.status().is_success() {
                return Err(StoreError::Unavailable);
            }
            response
                .bytes()
                .await
                .map(|value| value.to_vec())
                .map_err(|_| StoreError::Unavailable)
        })
    }
}

fn append_path(url: &mut Url, bucket: &str, key: &str) -> Result<(), StoreError> {
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| StoreError::Unavailable)?;
    segments.pop_if_empty();
    if !bucket.is_empty() {
        segments.push(bucket);
    }
    for segment in key.split('/') {
        segments.push(segment);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::time::{Duration, UNIX_EPOCH};

    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    use super::*;

    fn environment(endpoint: String, use_path_style: bool, secret: &str) -> S3Environment {
        S3Environment {
            bucket: "isolate-content".to_owned(),
            region: "test-region-1".to_owned(),
            endpoint,
            access_key_id: "test-access-key".to_owned(),
            secret_access_key: secret.to_owned(),
            session_token: Some("test-session-token".to_owned()),
            use_path_style,
        }
    }

    fn store(environment: S3Environment) -> S3ObjectStore {
        S3ObjectStore {
            client: Client::new(),
            environment,
            signing_time: Arc::new(|| UNIX_EPOCH + Duration::from_secs(1_700_000_000)),
        }
    }

    #[test]
    fn cloud_environment_aliases_and_session_credentials_are_read() {
        let values = HashMap::from([
            ("AWS_BUCKET", "isolate-content"),
            ("AWS_ENDPOINT", "https://legacy-endpoint.example"),
            ("AWS_ENDPOINT_URL", "https://preferred-endpoint.example"),
            ("AWS_DEFAULT_REGION", "fallback-region"),
            ("AWS_REGION", "preferred-region"),
            ("AWS_ACCESS_KEY_ID", "test-access-key"),
            ("AWS_SECRET_ACCESS_KEY", "test-secret-key"),
            ("AWS_SESSION_TOKEN", "test-session-token"),
            ("AWS_USE_PATH_STYLE_ENDPOINT", "true"),
        ]);
        let config = read_s3_environment(|key| {
            values
                .get(key)
                .map(ToString::to_string)
                .ok_or(std::env::VarError::NotPresent)
        });
        assert_eq!(config.bucket, "isolate-content");
        assert_eq!(config.endpoint, "https://preferred-endpoint.example");
        assert_eq!(config.region, "preferred-region");
        assert_eq!(config.session_token.as_deref(), Some("test-session-token"));
        assert!(config.use_path_style);

        let fallback = HashMap::from([
            ("AWS_ENDPOINT", "https://fallback-endpoint.example"),
            ("AWS_DEFAULT_REGION", "fallback-region"),
        ]);
        let config = read_s3_environment(|key| {
            fallback
                .get(key)
                .map(ToString::to_string)
                .ok_or(std::env::VarError::NotPresent)
        });
        assert_eq!(config.endpoint, "https://fallback-endpoint.example");
        assert_eq!(config.region, "fallback-region");
    }

    #[test]
    fn signed_get_uses_path_style_region_credentials_token_and_secret() {
        let first = store(environment(
            "http://localhost:3456/base".to_owned(),
            true,
            "first-test-secret",
        ))
        .signed_request("sites/example.com/message.txt")
        .expect("signed request");
        assert_eq!(
            first.url().as_str(),
            "http://localhost:3456/base/isolate-content/sites/example.com/message.txt"
        );
        let authorization = first.headers()["authorization"].to_str().unwrap();
        assert!(authorization
            .contains("Credential=test-access-key/20231114/test-region-1/s3/aws4_request"));
        assert_eq!(
            first.headers()["x-amz-security-token"],
            "test-session-token"
        );
        assert_eq!(first.headers()["x-amz-date"], "20231114T221320Z");

        let second = store(environment(
            "http://localhost:3456/base".to_owned(),
            true,
            "different-test-secret",
        ))
        .signed_request("sites/example.com/message.txt")
        .expect("signed request with different secret");
        assert_ne!(
            authorization,
            second.headers()["authorization"].to_str().unwrap(),
            "changing the signing secret must change the signature"
        );

        let virtual_hosted = store(environment(
            "http://localhost:3456/base".to_owned(),
            false,
            "first-test-secret",
        ))
        .signed_request("default/message.txt")
        .expect("virtual-hosted signed request");
        assert_eq!(
            virtual_hosted.url().host_str(),
            Some("isolate-content.localhost")
        );
        assert_eq!(virtual_hosted.url().path(), "/base/default/message.txt");
    }

    async fn endpoint_once(
        status: &str,
        body: &'static [u8],
    ) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let status = status.to_owned();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8_192];
            let read = stream.read(&mut request).await.unwrap();
            request.truncate(read);
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(response.as_bytes()).await.unwrap();
            stream.write_all(body).await.unwrap();
            String::from_utf8(request).unwrap()
        });
        (format!("http://{address}"), handle)
    }

    #[tokio::test]
    async fn object_store_executes_signed_gets_and_classifies_missing_objects() {
        let (endpoint, captured) = endpoint_once("200 OK", b"object bytes").await;
        let object_store = store(environment(endpoint, true, "test-secret"));
        assert_eq!(
            object_store.get("default/message.txt").await.unwrap(),
            b"object bytes"
        );
        let request = captured.await.unwrap();
        assert!(request.starts_with("GET /isolate-content/default/message.txt HTTP/1.1"));
        assert!(request
            .to_ascii_lowercase()
            .contains("authorization: aws4-hmac-sha256"));

        let (endpoint, _) = endpoint_once("404 Not Found", b"").await;
        let object_store = store(environment(endpoint, true, "test-secret"));
        assert_eq!(
            object_store.get("missing.txt").await,
            Err(StoreError::Missing)
        );
    }
}
