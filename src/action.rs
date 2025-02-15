use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "action", content = "payload", rename_all = "lowercase")]
pub enum Action {
    Reload,
    Stop,
    Transfer(TransferPayload),
    Kick(KickByIdPayload),
    GetPlayer(GetPlayerPayload),
    Execute(ExecuteCommandPayload),
    ExecuteShell(ExecuteShellCommandPayload),
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

#[derive(Debug, Deserialize, Hash, Eq, PartialEq)]
pub struct GetPlayerPayload {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteCommandPayload {
    pub command: String,
    pub result: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExecuteShellCommandPayload {
    pub main_command: String,
    pub args: Vec<String>,
    pub result: bool,
}
