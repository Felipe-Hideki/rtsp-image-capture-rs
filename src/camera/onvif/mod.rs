pub mod services;
use std::collections::HashMap;

use onvif::{schema::transport, soap::client::Credentials};
use services::{ClientWrapper, ManagementClient};
use url::Url;

const DEVICE_MGMT_PATH: &str = "/onvif/device_service";
const ONVIF_PORT: &str = "8899";

#[derive(Debug)]
pub enum OnvifError {
    UrlParseError,
    TransportError(transport::Error),
    UnsetToken,
    EmptyProfileList,
    ServiceNotFound(String),
}

pub struct OnvifHelper {
    onvif_url: Url,
    creds: Option<Credentials>,
    services_url: HashMap<String, Url>,
}

impl OnvifHelper {
    pub fn new(ip_address: &str) -> Result<Self, OnvifError> {
        let onvif_url = Url::parse(&("http://".to_string() + ip_address + ":" + ONVIF_PORT))
            .map_err(|_| OnvifError::UrlParseError)?;
        Ok(Self {
            onvif_url,
            creds: None,
            services_url: HashMap::new(),
        })
    }

    pub fn with_credentials(mut self, user: &str, pass: &str) -> Self {
        self.creds = Some(Credentials {
            username: user.to_string(),
            password: pass.to_string(),
        });
        self
    }

    pub async fn update_services_url(&mut self, force: bool) -> Result<(), OnvifError> {
        if !self.services_url.is_empty() && !force {
            return Ok(());
        }

        let mgmt_url = self.get_mgmt_url()?;

        let mgmt_cli = ManagementClient::connect(&mgmt_url, self.creds.clone());
        self.services_url
            .entry("management".to_string())
            .or_insert(mgmt_url);

        let resp = mgmt_cli.get_capabilities().await?;
        macro_rules! add_to_hashmap {
            ($field_name:ident) => {
                if resp.capabilities.$field_name.len() > 0 {
                    let v = Url::parse(&resp.capabilities.$field_name[0].x_addr.to_string())
                        .map_err(|_| OnvifError::UrlParseError)?;
                    self.services_url
                        .entry(stringify!($field_name).to_string())
                        .and_modify(|e| *e = v.clone())
                        .or_insert(v);
                }
            };
        }
        add_to_hashmap!(analytics);
        add_to_hashmap!(device);
        add_to_hashmap!(events);
        add_to_hashmap!(imaging);
        add_to_hashmap!(media);
        add_to_hashmap!(ptz);

        Ok(())
    }

    pub fn get_service_urls(&self) -> &HashMap<String, Url> {
        &self.services_url
    }

    pub async fn get_service<T: ClientWrapper>(&mut self) -> Result<T, OnvifError> {
        self.update_services_url(false).await?;
        let services = self.get_service_urls();
        let service_name = services
            .get(&T::get_service_name())
            .ok_or(OnvifError::ServiceNotFound(T::get_service_name()))?;
        Ok(T::connect(service_name, self.creds.clone()))
    }

    fn get_mgmt_url(&self) -> Result<Url, OnvifError> {
        self.onvif_url
            .join(DEVICE_MGMT_PATH)
            .map_err(|_| OnvifError::UrlParseError)
    }
}
