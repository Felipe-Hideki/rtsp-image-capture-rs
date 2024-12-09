use onvif::{
    schema::{
        self,
        onvif::{Profile, ReferenceToken, StreamType, Transport, TransportProtocol},
    },
    soap::client::{Client, ClientBuilder, Credentials},
};
use url::Url;

use super::OnvifError;

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

    pub async fn with_first_profile_token(self) -> Result<Self, OnvifError> {
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
        let req = schema::media::SetSynchronizationPoint {
            profile_token: ReferenceToken {
                0: self
                    .profile_token
                    .as_ref()
                    .ok_or(OnvifError::UnsetToken)?
                    .0
                    .to_string(),
            },
        };
        schema::media::set_synchronization_point(&self.inner, &req)
            .await
            .map_err(|e| OnvifError::TransportError(e))
            .map(|_| ())
    }

    pub async fn get_stream_uri(&self) -> Result<Url, OnvifError> {
        let req = schema::media::GetStreamUri {
            stream_setup: schema::onvif::StreamSetup {
                stream: StreamType::RtpUnicast,
                transport: Transport {
                    protocol: TransportProtocol::Rtsp,
                    tunnel: Vec::new(),
                },
            },
            profile_token: ReferenceToken {
                0: self
                    .profile_token
                    .as_ref()
                    .ok_or(OnvifError::UnsetToken)?
                    .0
                    .to_string(),
            },
        };

        schema::media::get_stream_uri(&self.inner, &req)
            .await
            .map_err(|e| OnvifError::TransportError(e))
            .map(|u| Url::parse(&u.media_uri.uri).map_err(|_| OnvifError::UrlParseError))?
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
