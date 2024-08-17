use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", content = "payload", rename_all = "lowercase")]
pub enum Action {
    Reload,
    Stop,
    Transfer(TransferPayload),
    Kick(KickByIdPayload),
    Execute(ExecuteCommandPayload),
}

#[derive(Debug, Deserialize)]
pub struct TransferPayload {
    pub player: String,
    pub host: String,
    pub port: u16,
}

#[derive(Debug, Deserialize)]
pub struct KickByIdPayload {
    pub player: String,
    pub reason: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandPayload {
    pub command: String,
    pub result: bool,
}
