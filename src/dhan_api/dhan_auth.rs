pub(crate) use super::dhan_oauth::source::get_dhan_credentials;

#[allow(dead_code)]
pub(crate) struct DhanCredentials {
    pub(crate) client_id: String,
    pub(crate) api_key: String,
    pub(crate) access_token: String,
}
