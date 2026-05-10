use std::{borrow::Cow, collections::HashMap, fmt::Display};

use async_stream::try_stream;
use async_trait::async_trait;
use aws_config::{BehaviorVersion, Region};
use aws_credential_types::Credentials;
use aws_sdk_dynamodb::{
    error::SdkError,
    types::{
        AttributeDefinition, AttributeValue, BillingMode, KeySchemaElement, KeyType,
        ScalarAttributeType, TimeToLiveSpecification,
    },
    Client,
};
use futures::stream::BoxStream;
use lazy_regex::regex_captures;

use crate::{
    backend_api::{CuttleBackend, PutOptions},
    common::{get_system_time, CuttlestoreError},
};

fn dynamo_err<E: Display>(err: E) -> CuttlestoreError {
    CuttlestoreError::DynamoDBError(err.to_string())
}

const KEY_ATTR: &str = "key";
const VALUE_ATTR: &str = "value";
const TTL_ATTR: &str = "live_until";

pub(crate) struct DynamoDBBackend {
    client: Client,
    table: String,
}

impl DynamoDBBackend {
    async fn new(
        region: &str,
        table: &str,
        args: HashMap<&str, &str>,
    ) -> Result<Box<Self>, CuttlestoreError> {
        let mut loader = aws_config::defaults(BehaviorVersion::latest())
            .region(Region::new(region.to_string()));

        if let Some(endpoint) = args.get("endpoint") {
            loader = loader.endpoint_url(*endpoint);
        }

        match (args.get("access_key"), args.get("secret_key")) {
            (Some(access), Some(secret)) => {
                loader = loader.credentials_provider(Credentials::from_keys(*access, *secret, None));
            }
            _ => {
                // For DynamoDB Local, the SDK still requires credentials to be
                // present even though they are ignored by the server. If the
                // user did not configure any, fall back to test credentials so
                // local development works without setting AWS_* env vars.
                if args.contains_key("endpoint") {
                    loader = loader.credentials_provider(Credentials::from_keys(
                        "cuttlestore-local",
                        "cuttlestore-local",
                        None,
                    ));
                }
            }
        }

        let config = loader.load().await;
        let client = Client::new(&config);

        let backend = DynamoDBBackend {
            client,
            table: table.to_string(),
        };

        backend.ensure_table().await?;
        Ok(Box::new(backend))
    }

    async fn ensure_table(&self) -> Result<(), CuttlestoreError> {
        match self
            .client
            .describe_table()
            .table_name(&self.table)
            .send()
            .await
        {
            Ok(_) => return Ok(()),
            Err(SdkError::ServiceError(err)) if err.err().is_resource_not_found_exception() => {}
            Err(err) => return Err(dynamo_err(err)),
        };

        self.client
            .create_table()
            .table_name(&self.table)
            .billing_mode(BillingMode::PayPerRequest)
            .attribute_definitions(
                AttributeDefinition::builder()
                    .attribute_name(KEY_ATTR)
                    .attribute_type(ScalarAttributeType::S)
                    .build()
                    .map_err(dynamo_err)?,
            )
            .key_schema(
                KeySchemaElement::builder()
                    .attribute_name(KEY_ATTR)
                    .key_type(KeyType::Hash)
                    .build()
                    .map_err(dynamo_err)?,
            )
            .send()
            .await
            .map_err(dynamo_err)?;

        // Best-effort: enable DynamoDB's native TTL on the live_until
        // attribute. DynamoDB deletes expired items on its own schedule
        // (typically within 48 hours), so `get` and `scan` still filter by
        // `live_until` for correctness.
        if let Ok(spec) = TimeToLiveSpecification::builder()
            .attribute_name(TTL_ATTR)
            .enabled(true)
            .build()
        {
            let _ = self
                .client
                .update_time_to_live()
                .table_name(&self.table)
                .time_to_live_specification(spec)
                .send()
                .await;
        }

        Ok(())
    }
}

#[async_trait]
impl CuttleBackend for DynamoDBBackend {
    async fn new(conn: &str) -> Option<Result<Box<Self>, CuttlestoreError>> {
        let (_, region, table, args) =
            regex_captures!(r#"^dynamodb://([^/]+)/([^?]+)[?]?(.*)"#, conn)?;

        let arg_pairs: HashMap<&str, &str> = args
            .split('&')
            .filter(|p| !p.is_empty())
            .flat_map(|pair| pair.split_once('='))
            .collect();

        Some(DynamoDBBackend::new(region, table, arg_pairs).await)
    }

    fn requires_cleaner(&self) -> bool {
        // DynamoDB has built-in TTL cleanup. We also filter expired items in
        // `get` and `scan` for correctness, so an external cleaner is not
        // needed.
        false
    }

    fn name(&self) -> &'static str {
        "dynamodb"
    }

    async fn get<'a>(&self, key: Cow<'a, str>) -> Result<Option<Vec<u8>>, CuttlestoreError> {
        let response = self
            .client
            .get_item()
            .table_name(&self.table)
            .key(KEY_ATTR, AttributeValue::S(key.to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;

        let Some(item) = response.item else {
            return Ok(None);
        };

        if let Some(AttributeValue::N(live_until)) = item.get(TTL_ATTR) {
            if let Ok(live_until) = live_until.parse::<u64>() {
                if live_until < get_system_time() {
                    self.delete(key).await?;
                    return Ok(None);
                }
            }
        }

        match item.get(VALUE_ATTR) {
            Some(AttributeValue::B(blob)) => Ok(Some(blob.clone().into_inner())),
            _ => Ok(None),
        }
    }

    async fn put<'a>(
        &self,
        key: Cow<'a, str>,
        value: &[u8],
        options: PutOptions,
    ) -> Result<(), CuttlestoreError> {
        let mut request = self
            .client
            .put_item()
            .table_name(&self.table)
            .item(KEY_ATTR, AttributeValue::S(key.to_string()))
            .item(
                VALUE_ATTR,
                AttributeValue::B(aws_sdk_dynamodb::primitives::Blob::new(value.to_vec())),
            );

        if let Some(ttl) = options.ttl {
            let live_until = get_system_time() + ttl;
            request = request.item(TTL_ATTR, AttributeValue::N(live_until.to_string()));
        }

        request.send().await.map_err(dynamo_err)?;
        Ok(())
    }

    async fn delete<'a>(&self, key: Cow<'a, str>) -> Result<(), CuttlestoreError> {
        self.client
            .delete_item()
            .table_name(&self.table)
            .key(KEY_ATTR, AttributeValue::S(key.to_string()))
            .send()
            .await
            .map_err(dynamo_err)?;
        Ok(())
    }

    async fn scan(
        &self,
    ) -> Result<BoxStream<Result<(String, Vec<u8>), CuttlestoreError>>, CuttlestoreError> {
        let client = self.client.clone();
        let table = self.table.clone();

        Ok(Box::pin(try_stream! {
            let mut last_key: Option<HashMap<String, AttributeValue>> = None;
            loop {
                let mut request = client.scan().table_name(&table);
                if let Some(start) = last_key.take() {
                    request = request.set_exclusive_start_key(Some(start));
                }
                let response = request.send().await.map_err(dynamo_err)?;
                let now = get_system_time();
                if let Some(items) = response.items {
                    for item in items {
                        let key = match item.get(KEY_ATTR) {
                            Some(AttributeValue::S(k)) => k.clone(),
                            _ => continue,
                        };
                        if let Some(AttributeValue::N(live_until)) = item.get(TTL_ATTR) {
                            if let Ok(live_until) = live_until.parse::<u64>() {
                                if live_until < now {
                                    client
                                        .delete_item()
                                        .table_name(&table)
                                        .key(KEY_ATTR, AttributeValue::S(key.clone()))
                                        .send()
                                        .await
                                        .map_err(dynamo_err)?;
                                    continue;
                                }
                            }
                        }
                        let value = match item.get(VALUE_ATTR) {
                            Some(AttributeValue::B(blob)) => blob.clone().into_inner(),
                            _ => continue,
                        };
                        yield (key, value);
                    }
                }
                match response.last_evaluated_key {
                    Some(key) if !key.is_empty() => last_key = Some(key),
                    _ => break,
                }
            }
        }))
    }
}
