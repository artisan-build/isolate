package main

import (
	"context"
	"errors"
	"io"
	"strconv"

	"github.com/aws/aws-sdk-go-v2/aws"
	"github.com/aws/aws-sdk-go-v2/credentials"
	"github.com/aws/aws-sdk-go-v2/service/s3"
	"github.com/aws/smithy-go"
)

type s3Getter interface {
	GetObject(context.Context, *s3.GetObjectInput, ...func(*s3.Options)) (*s3.GetObjectOutput, error)
}

type s3ObjectStore struct {
	client s3Getter
	bucket string
}

func (s *s3ObjectStore) Get(ctx context.Context, key string) ([]byte, error) {
	output, err := s.client.GetObject(ctx, &s3.GetObjectInput{
		Bucket: aws.String(s.bucket),
		Key:    aws.String(key),
	})
	if err != nil {
		var apiError smithy.APIError
		if errors.As(err, &apiError) && (apiError.ErrorCode() == "NoSuchKey" || apiError.ErrorCode() == "NotFound") {
			return nil, errObjectNotFound
		}

		return nil, err
	}
	defer output.Body.Close()

	value, err := io.ReadAll(output.Body)
	if err != nil {
		return nil, err
	}

	return value, nil
}

type missingObjectStore struct{}

func (missingObjectStore) Get(context.Context, string) ([]byte, error) {
	return nil, errObjectNotFound
}

type failedObjectStore struct {
	err error
}

func (s failedObjectStore) Get(context.Context, string) ([]byte, error) {
	return nil, s.err
}

type s3Environment struct {
	bucket          string
	region          string
	endpoint        string
	accessKeyID     string
	secretAccessKey string
	sessionToken    string
	usePathStyle    bool
}

func readS3Environment(getenv func(string) string) s3Environment {
	region := getenv("AWS_REGION")
	if region == "" {
		region = getenv("AWS_DEFAULT_REGION")
	}
	if region == "" {
		region = "us-east-1"
	}

	endpoint := getenv("AWS_ENDPOINT_URL")
	if endpoint == "" {
		endpoint = getenv("AWS_ENDPOINT")
	}

	usePathStyle, _ := strconv.ParseBool(getenv("AWS_USE_PATH_STYLE_ENDPOINT"))

	return s3Environment{
		bucket:          getenv("AWS_BUCKET"),
		region:          region,
		endpoint:        endpoint,
		accessKeyID:     getenv("AWS_ACCESS_KEY_ID"),
		secretAccessKey: getenv("AWS_SECRET_ACCESS_KEY"),
		sessionToken:    getenv("AWS_SESSION_TOKEN"),
		usePathStyle:    usePathStyle,
	}
}

func newObjectStore(getenv func(string) string) objectStore {
	environment := readS3Environment(getenv)
	if environment.bucket == "" {
		return missingObjectStore{}
	}
	if environment.accessKeyID == "" || environment.secretAccessKey == "" {
		return failedObjectStore{err: errors.New("object store credentials are not configured")}
	}

	configuration := aws.Config{
		Region: environment.region,
		Credentials: aws.NewCredentialsCache(credentials.NewStaticCredentialsProvider(
			environment.accessKeyID,
			environment.secretAccessKey,
			environment.sessionToken,
		)),
	}

	client := s3.NewFromConfig(configuration, func(options *s3.Options) {
		options.UsePathStyle = environment.usePathStyle
		if environment.endpoint != "" {
			options.BaseEndpoint = aws.String(environment.endpoint)
		}
	})

	return &s3ObjectStore{client: client, bucket: environment.bucket}
}
