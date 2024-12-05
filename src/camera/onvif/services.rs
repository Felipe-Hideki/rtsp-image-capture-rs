use onvif::{
    schema::{
        self,
        onvif::{Profile, ReferenceToken},
    },
    soap::client::{Client, ClientBuilder, Credentials},
};
use url::Url;

use super::{OnvifError, OnvifHelper};

pub trait ClientWrapper {
    fn connect(endpoint: &Url, auth: Option<Credentials>) -> Self;
    fn get_service_name() -> String;
}

#[derive(Clone)]
pub struct ManagementClient {
    inner: Client,
}

impl ManagementClient {
    pub async fn get_capabilities(
        &self,
    ) -> Result<schema::devicemgmt::GetCapabilitiesResponse, OnvifError> {
        schema::devicemgmt::get_capabilities(&self.inner, &Default::default())
            .await
            .map_err(|e| OnvifError::TransportError(e))
    }
}
impl ClientWrapper for ManagementClient {
    fn connect(endpoint: &Url, auth: Option<Credentials>) -> Self {
        let cli = ClientBuilder::new(&endpoint).credentials(auth).build();
        Self { inner: cli }
    }
    fn get_service_name() -> String {
        "management".to_string()
    }
}

pub struct MediaClient {
    inner: Client,
    profile_token: Option<ReferenceToken>,
}

impl MediaClient {
    // pub fn with_string_token(&mut self, token: impl Into<String>) {
    //     self.profile_token = Some(ReferenceToken { 0: token.into() })
    // }

    pub async fn with_first_profile(self) -> Result<Self, OnvifError> {
        let mut profiles = self.get_profiles().await?;

        if profiles.len() == 0 {
            return Err(OnvifError::EmptyProfileList);
        }
        Ok(self.with_token(profiles.remove(0).token))
    }

    pub fn with_token(mut self, token: ReferenceToken) -> Self {
        self.profile_token = Some(token);
        self
    }

    pub async fn get_profiles(&self) -> Result<Vec<Profile>, OnvifError> {
        schema::media::get_profiles(&self.inner, &Default::default())
            .await
            .map_err(|e| OnvifError::TransportError(e))
            .map(|r| r.profiles)
    }

    pub async fn sync_iframe(&self) -> Result<(), OnvifError> {
        if self.profile_token.is_none() {
            return Err(OnvifError::UnsetToken);
        }
        let req = schema::media::SetSynchronizationPoint {
            profile_token: ReferenceToken {
                0: self.profile_token.as_ref().unwrap().0.to_string(),
            },
        };
        schema::media::set_synchronization_point(&self.inner, &req)
            .await
            .map_err(|e| OnvifError::TransportError(e))
            .map(|_| ())
    }
}

impl Clone for MediaClient {
    fn clone(&self) -> Self {
        let cloned_token = self.profile_token.as_ref().map(|p_token| ReferenceToken {
            0: p_token.0.to_string(),
        });

        Self {
            inner: self.inner.clone(),
            profile_token: cloned_token,
        }
    }
}

impl ClientWrapper for MediaClient {
    fn connect(endpoint: &Url, auth: Option<Credentials>) -> Self {
        let cli = ClientBuilder::new(&endpoint).credentials(auth).build();
        Self {
            inner: cli,
            profile_token: None,
        }
    }
    fn get_service_name() -> String {
        "media".to_string()
    }
}
