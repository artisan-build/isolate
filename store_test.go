package main

import (
	"context"
	"io"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/aws/smithy-go"
)

type fakeS3Getter struct {
	output *s3.GetObjectOutput
	err    error
}

func (g fakeS3Getter) GetObject(
	context.Context,
	*s3.GetObjectInput,
	...func(*s3.Options),
) (*s3.GetObjectOutput, error) {
	return g.output, g.err
}

func TestReadS3EnvironmentSupportsCloudVariableNames(t *testing.T) {
	values := map[string]string{
		"AWS_BUCKET":                  "isolate-content",
		"AWS_ENDPOINT":                "https://legacy-endpoint.example",
		"AWS_ENDPOINT_URL":            "https://preferred-endpoint.example",
		"AWS_DEFAULT_REGION":          "fallback-region",
		"AWS_REGION":                  "preferred-region",
		"AWS_ACCESS_KEY_ID":           "test-access-key",
		"AWS_SECRET_ACCESS_KEY":       "test-secret-key",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
	environment := readS3Environment(func(key string) string { return values[key] })

	if environment.bucket != "isolate-content" {
		t.Errorf("bucket = %q", environment.bucket)
	}
	if environment.endpoint != "https://preferred-endpoint.example" {
		t.Errorf("endpoint = %q, want AWS_ENDPOINT_URL", environment.endpoint)
	}
	if environment.region != "preferred-region" {
		t.Errorf("region = %q, want AWS_REGION", environment.region)
	}
	if !environment.usePathStyle {
		t.Error("path-style endpoint setting is false, want true")
	}
	if environment.accessKeyID == "" || environment.secretAccessKey == "" {
		t.Error("credentials were not read")
	}

	fallbackValues := map[string]string{
		"AWS_BUCKET":         "fallback-bucket",
		"AWS_ENDPOINT":       "https://fallback-endpoint.example",
		"AWS_DEFAULT_REGION": "fallback-region",
	}
	fallback := readS3Environment(func(key string) string { return fallbackValues[key] })
	if fallback.endpoint != "https://fallback-endpoint.example" || fallback.region != "fallback-region" {
		t.Errorf("fallback endpoint/region = %q/%q", fallback.endpoint, fallback.region)
	}
}

func TestS3ObjectStoreReadsBytesAndClassifiesMissingKeys(t *testing.T) {
	store := &s3ObjectStore{
		client: fakeS3Getter{output: &s3.GetObjectOutput{Body: io.NopCloser(strings.NewReader("object bytes"))}},
		bucket: "test-bucket",
	}
	value, err := store.Get(context.Background(), "default/message.txt")
	if err != nil || string(value) != "object bytes" {
		t.Fatalf("Get = %q, %v", value, err)
	}

	store.client = fakeS3Getter{err: &smithy.GenericAPIError{Code: "NoSuchKey", Message: "missing"}}
	if _, err := store.Get(context.Background(), "missing"); err != errObjectNotFound {
		t.Errorf("missing Get error = %v, want errObjectNotFound", err)
	}
}

func TestNewObjectStoreUsesInjectedEndpointCredentialsAndPathStyle(t *testing.T) {
	server := httptest.NewServer(http.HandlerFunc(func(response http.ResponseWriter, request *http.Request) {
		if request.URL.Path != "/isolate-content/default/message.txt" {
			t.Errorf("request path = %q, want path-style bucket and key", request.URL.Path)
		}
		if !strings.HasPrefix(request.Header.Get("Authorization"), "AWS4-HMAC-SHA256 ") {
			t.Error("request is not signed with the injected credentials")
		}
		_, _ = response.Write([]byte("endpoint test object"))
	}))
	defer server.Close()

	values := map[string]string{
		"AWS_BUCKET":                  "isolate-content",
		"AWS_ENDPOINT_URL":            server.URL,
		"AWS_REGION":                  "test-region-1",
		"AWS_ACCESS_KEY_ID":           "test-access-key",
		"AWS_SECRET_ACCESS_KEY":       "test-secret-key",
		"AWS_USE_PATH_STYLE_ENDPOINT": "true",
	}
	store := newObjectStore(func(key string) string { return values[key] })
	ctx, cancel := context.WithTimeout(context.Background(), time.Second)
	defer cancel()

	value, err := store.Get(ctx, "default/message.txt")
	if err != nil {
		t.Fatalf("Get through configured endpoint: %v", err)
	}
	if string(value) != "endpoint test object" {
		t.Errorf("Get body = %q, want endpoint response", value)
	}
}
