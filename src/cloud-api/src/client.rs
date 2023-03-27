use std::fmt::Display;
use std::str::FromStr;

use anyhow::{bail, ensure, Context, Result};
use reqwest::{Url, Method, RequestBuilder, Error};
use serde::{Deserialize, Serialize};
use once_cell::sync::Lazy;

use mz_frontegg_auth as frontegg_auth;

pub static DEFAULT_ENDPOINT: Lazy<Url> =
    Lazy::new(|| "https://cloud.materialize.com".parse().unwrap());

/// Configures a `Client`.
pub struct ClientBuilder {
    endpoint: Url,
}

pub struct ClientConfig {
    frontegg_client: frontegg_auth::Client,
}

impl Default for ClientBuilder {
    fn default() -> ClientBuilder {
        ClientBuilder {
            endpoint: DEFAULT_ENDPOINT.clone(),
        }
    }
}

impl ClientBuilder {
    /// Overrides the default endpoint.
    pub fn endpoint(mut self, url: Url) -> ClientBuilder {
        self.endpoint = url;
        self
    }

    /// Creates a [`Client`] that incorporates the optional parameters
    /// configured on the builder and the specified required parameters.
    pub fn build(self, config: ClientConfig) -> Client {
        Client {
            frontegg_client: config.frontegg_client,
            auth: None,
            endpoint: self.endpoint,
        }
    }
}

pub struct Auth {
    token: String,
}

pub struct Client {
    frontegg_client: frontegg_auth::Client,
    auth: Option<Auth>,
    endpoint: Url,
}

impl Client {
    /// Authenticates with the server, if not already authenticated,
    /// and returns the authentication token.
    pub async fn auth_token(&mut self) -> Result<String> {
        if self.auth.is_none() {
            // Authenticate if not authenticated.
            // TODO: Implement authentication logic here.
            self.auth = Some(Auth {
                token: String::from("your_token_here"),
            });
        }
        Ok(self.auth.as_ref().unwrap().token.clone())
    }

    pub async fn list_cloud_regions(&self, valid_profile: &str) -> Result<Vec<CloudProviderAndRegion>> {
        // TODO: Run requests in parallel
        let mut cloud_providers_and_regions: Vec<CloudProviderAndRegion> = Vec::new();

        for cloud_provider in cloud_providers {
            let cloud_provider_region_details =
                get_cloud_provider_region_details(client, cloud_provider, valid_profile)
                    .await
                    .with_context(|| "Retrieving region details.")?;
            match cloud_provider_region_details.get(0) {
                Some(region) => cloud_providers_and_regions.push(CloudProviderAndRegion {
                    cloud_provider: cloud_provider.clone(),
                    region: Some(region.to_owned()),
                }),
                None => cloud_providers_and_regions.push(CloudProviderAndRegion {
                    cloud_provider: cloud_provider.clone(),
                    region: None,
                }),
            }
        }
        Ok(cloud_providers_and_regions)
    }

    pub async fn get_environment(&self, region_name: &str) -> Result<Environment> {
        let environment_details = region_environment_details(client, region, valid_profile)
            .await
            .with_context(|| "Environment unavailable")?;
        let environment_list = environment_details.with_context(|| "Environment unlisted")?;
        let environment = environment_list
            .get(0)
            .with_context(|| "Missing environment")?;

        Ok(environment.to_owned())
    }

    pub async fn get_all_environments(&self) -> Result<Environment> {
        let region = get_provider_region(client, valid_profile, cloud_provider_region)
            .await
            .with_context(|| "Retrieving region data.")?;

        let environment = get_region_environment(client, valid_profile, &region)
            .await
            .with_context(|| "Retrieving environment data")?;

        Ok(environment)
    }


    pub async fn create_environment(&self) -> Result<Region, reqwest::Error> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body {
            #[serde(skip_serializing_if = "Option::is_none")]
            environmentd_image_ref: Option<String>,
            #[serde(skip_serializing_if = "Vec::is_empty")]
            environmentd_extra_args: Vec<String>,
        }

        let body = Body {
            environmentd_image_ref: version.map(|v| match v.split_once(':') {
                None => format!("materialize/environmentd:{v}"),
                Some((user, v)) => format!("{user}/environmentd:{v}"),
            }),
            environmentd_extra_args,
        };

        client
            .post(format!("{:}/api/environmentassignment", cloud_provider.api_url).as_str())
            .authenticate(&valid_profile.frontegg_auth)
            .json(&body)
            .send()
            .await?
            .json::<Region>()
            .await
    }

    async fn request(&self, method: Method, url: &str) -> Result<RequestBuilder> {
        // Makes a request using the frontegg client's authentication.
        let token = self.auth_token().await?;
        let request = self.frontegg_client.request(method, url)
            .bearer_auth(token);
        Ok(request)
    }
}

// TODO: nice error type. Use `rust_frontegg` for inspiration.
