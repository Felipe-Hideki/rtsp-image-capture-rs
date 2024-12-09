use url::Url;

use super::SessionError;

pub struct SessionUrlBuilder {
    user: Option<String>,
    password: Option<String>,
    ip_address: String,
    port: String,
}

impl SessionUrlBuilder {
    pub fn with_user(mut self, val: Option<String>) -> Self {
        self.user = val;
        self
    }
    pub fn with_password(mut self, val: Option<String>) -> Self {
        self.password = val;
        self
    }
    pub fn with_ip_address(mut self, val: String) -> Self {
        self.ip_address = val;
        self
    }
    pub fn with_port(mut self, val: String) -> Self {
        self.port = val;
        self
    }

    pub fn build(self) -> Result<Url, SessionError> {
        if self.ip_address.is_empty() {
            return Err(SessionError::UnsetParameter(
                "Necessary parameter unset: ip_address".to_string(),
            ));
        }
        let mut builder = format!("rtsp://{}:{}", self.ip_address, self.port);

        if let (Some(user), Some(pass)) = (self.user, self.password) {
            builder += &format!("/user={}&password={}", &user, &pass);
        }

        Url::parse(&builder).map_err(|_| SessionError::UrlParseError)
    }
}

impl Default for SessionUrlBuilder {
    fn default() -> Self {
        Self {
            user: None,
            password: None,
            ip_address: String::new(),
            port: String::from("554"),
        }
    }
}
