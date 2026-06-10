use crate::{config::Config, AppError, AppResult};
use aws_config::BehaviorVersion;
use aws_sdk_s3::{config::Region, primitives::ByteStream, Client};
use std::time::Duration;

#[derive(Clone)]
pub struct RustFsClient {
    inner: Client,
    bucket: String,
}

impl RustFsClient {
    pub async fn new(config: &Config) -> AppResult<Self> {
        let shared_config = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new("us-east-1"))
            .endpoint_url(&config.rustfs_endpoint)
            .load()
            .await;

        Ok(Self {
            inner: Client::new(&shared_config),
            bucket: config.rustfs_bucket.clone(),
        })
    }

    pub async fn upload(&self, key: &str, data: Vec<u8>, content_type: &str) -> AppResult<()> {
        self.inner
            .put_object()
            .bucket(&self.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from(data))
            .send()
            .await
            .map_err(|error| AppError::other(error.to_string()))?;

        Ok(())
    }

    pub async fn download(&self, key: &str) -> AppResult<Vec<u8>> {
        let response = self
            .inner
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .send()
            .await
            .map_err(|error| AppError::other(error.to_string()))?;
        let bytes = response
            .body
            .collect()
            .await
            .map_err(|error| AppError::other(error.to_string()))?
            .into_bytes();
        Ok(bytes.to_vec())
    }

    pub async fn presigned_url(&self, key: &str, expires_in: Duration) -> AppResult<String> {
        let presign_config = aws_sdk_s3::presigning::PresigningConfig::expires_in(expires_in)
            .map_err(|error| AppError::other(error.to_string()))?;
        let request = self
            .inner
            .get_object()
            .bucket(&self.bucket)
            .key(key)
            .presigned(presign_config)
            .await
            .map_err(|error| AppError::other(error.to_string()))?;

        Ok(request.uri().to_string())
    }
}
