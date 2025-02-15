pub mod action;
pub mod color;
pub mod consts;
pub mod log_level;
pub mod stream;

use json::{self, object};
use regex::Regex;
use std::env;
use std::io::Write;
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::thread;
use action::Action;
use color::Color;
use consts::LOG_PREFIX;
use log_level::LogLevel;
use stream::LogDelimiterStream;
use std::collections::HashMap;
use serde_json::Value;
use serde::{Deserialize, Serialize};
use crate::action::GetPlayerPayload;

lazy_static::lazy_static! {
    static ref ACTION_MESSAGE_REGEX: Regex = Regex::new(r".*\[Scripting\] bds_enhancer:(?P<json>\{.*\})").unwrap();
    static ref LOG_REGEX: Regex = Regex::new(&format!(r"{} (?P<level>(INFO|WARN|ERROR))\] ", LOG_PREFIX)).unwrap();
    static ref ON_JOIN_REGEX: Regex = Regex::new(r"Player connected: (?P<player>.+), xuid: (?P<xuid>\d+)").unwrap();
    static ref ON_SPAWN_REGEX: Regex = Regex::new(r"Player Spawned: (?P<player>.+) xuid: (?P<xuid>\d+), pfid: (?P<pfid>.+)").unwrap();
}


#[derive(Debug, Serialize, Deserialize)]
struct Player {
    deviceSessionId: String,
    name: String,
    xuid: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Response {
    command: String,
    result: Vec<Player>,
}

#[derive(Debug, Clone)]
struct PlayerInfo {
    name: String,
    device_id: String,
    xuid: String,
}

impl PlayerCache {
    fn new() -> Self {
        PlayerCache {
            players: HashMap::new(),
        }
    }

    fn add_player(&mut self, name: &str, device_id: &str, xuid: &str) {
        let player_info = PlayerInfo {
            name: name.to_string(),
            device_id: device_id.to_string(),
            xuid: xuid.to_string(),
        };
        self.players.insert(name.to_string(), player_info);
    }

    fn get_player_info(&self, name: &GetPlayerPayload) -> Option<&PlayerInfo> {
        self.players.get(name)
    }
}

struct PlayerCache {
    players: HashMap<String, PlayerInfo>, // プレイヤー名をキーにして情報を格納
}

fn handle_child_stdin(rx: Receiver<String>, mut child_stdin: ChildStdin) {
    loop {
        let input = rx.recv().unwrap();
        child_stdin
            .write_all(input.as_bytes())
            .expect("Failed to write to stdin");
    }
}

fn handle_stdin(child_stdin: Sender<String>) {
    let stdin = std::io::stdin();

    loop {
        let mut line = String::new();
        stdin.read_line(&mut line).unwrap();

        child_stdin.send(line).unwrap();
    }
}

fn get_log_level(log: &str) -> LogLevel {
    let level = LOG_REGEX
        .captures(log)
        .map(|caps| caps["level"].to_string())
        .unwrap_or("INFO".to_string());

    level.parse().unwrap()
}

fn parse_action(log: &str) -> Option<Action> {
    let caps = ACTION_MESSAGE_REGEX.captures(log)?;

    let json = caps.name("json").unwrap().as_str();
    serde_json::from_str(json).ok()?
}

fn handle_listd_log(log: &str, cache: &mut PlayerCache) {
    if let Some(index) = log.find("*###") {
        let log_after_prefix = &log[index + 4..];

        if let Ok(parsed) = serde_json::from_str::<Value>(log_after_prefix) {
            if parsed["command"] == "listd" {
                let empty_vec = vec![]; // 一時的なvecを変数に束縛
                let players = parsed["result"].as_array().unwrap_or(&empty_vec);

                for player in players {
                    let name = player["name"].as_str().unwrap_or("");
                    let device_id = player["deviceSessionId"].as_str().unwrap_or("");
                    let xuid = player["xuid"].as_str().unwrap_or("");

                    cache.add_player(name, device_id, xuid);
                }
            }
        }
    }
}

fn handle_action(child_stdin: &Sender<String>, action: Action, command_status: &mut CommandStatus, cache: &mut PlayerCache) {
    match action {
        Action::Transfer(arg) => execute_command(
            child_stdin,
            format!("transfer {} {} {}", arg.player, arg.host, arg.port),
        ),
        Action::Kick(arg) => {
            execute_command(child_stdin, format!("kick {} {}", arg.player, arg.reason))
        }
        Action::Reload => execute_command(child_stdin, "reload".to_string()),
        Action::Stop => execute_command(child_stdin, "stop".to_string()),
        Action::Execute(arg) => {
            if arg.result {
                command_status.waiting = true;
                command_status.command = arg.command.clone();
                command_status.scriptevent = "bds_enhancer:result".to_string();
            }
            execute_command(child_stdin, arg.command.to_string());
        }
        Action::GetPlayer(arg) => {
            if let Some(player) = cache.get_player_info(&arg) {
                let json_data = serde_json::json!({
                    "name": player.name,
                    "deviceId": player.device_id,
                    "xuid": player.xuid
                }).to_string();
                execute_command(
                    child_stdin,
                    format!("scriptevent system:playerinfo {}", json_data),
                );
            }
        }
        Action::ExecuteShell(arg) => {
            let result = execute_shell_command(&arg.main_command.clone(), arg.args.clone());
            match result {
                Ok(result) => {
                    for i in 0..result.trim().chars().count() / 1500 + 1 {
                        let result_tmp = result
                            .trim()
                            .chars()
                            .skip(i * 1500)
                            .take(1500)
                            .collect::<String>();
                        let result: json::JsonValue = object! {
                            "command" => arg.main_command.clone() + " " + &arg.args.clone().join(" "),
                            "result_message" => result_tmp.clone(),
                            "count" => i,
                            "end" => i == result.trim().chars().count() / 1500,
                            "err" => false,
                        };
                        execute_command(
                            child_stdin,
                            format!(
                                "scriptevent {} {}",
                                "bds_enhancer:shell_result",
                                result.dump()
                            ),
                        );
                    }
                }
                Err(e) => {
                    if arg.result {
                        let return_value = object! {
                            "command" => arg.main_command + " " + &arg.args.clone().join(" "),
                            "result_message" => format!("Error: {}", e),
                            "err" => true,
                        };
                        execute_command(
                            child_stdin,
                            format!(
                                "scriptevent {} {}",
                                "bds_enhancer:shell_result",
                                return_value.dump()
                            ),
                        );
                    }
                }
            }
        }
    }
}

fn get_player_info_and_send(name: &str, cache: &PlayerCache, child_stdin: &Sender<String>) {
    if let Some(player_info) = cache.get_player_info(name) {
        // プレイヤー情報を scriptevent コマンドで送信
        send_to_scriptevent(&player_info.name, &player_info.xuid, &player_info.device_id, child_stdin);
    } else {
        println!("Player not found: {}", name);
    }
}

fn send_to_scriptevent(player: &str, xuid: &str, device_id: &str, child_stdin: &Sender<String>) {
    // プレイヤー情報を scriptevent コマンドとして送信
    let command = format!("scriptevent system:playerinfo {{\"name\":\"{}\",\"xuid\": {},\"deviceId\":{}}}", player, xuid, device_id);
    execute_command(child_stdin, command);
}

fn custom_handler(log: &str, child_stdin: &Sender<String>) {
    if let Some(caps) = ON_JOIN_REGEX.captures(log) {
        let player = caps.name("player").unwrap().as_str();
        let xuid = caps.name("xuid").unwrap().as_str();
        execute_command(
            child_stdin,
            format!("scriptevent system:on_join {}|{}", player, xuid),
        );
    } else if let Some(caps) = ON_SPAWN_REGEX.captures(log) {
        let player = caps.name("player").unwrap().as_str();
        let xuid = caps.name("xuid").unwrap().as_str();
        let pfid = caps.name("pfid").unwrap().as_str();
        execute_command(
            child_stdin,
            format!("scriptevent system:on_spawn {}|{}|{}", player, xuid, pfid),
        );
    }
}

fn handle_child_stdout(
    child_stdin: Sender<String>,
    child_stdout: ChildStdout,
    mut command_status: &mut CommandStatus,
) {
    let logs = LogDelimiterStream::new(child_stdout);
    let mut stdout = std::io::stdout();

    for log in logs {
        if let Some(action) = parse_action(&log) {
            handle_action(&child_stdin, action, &mut command_status, &mut PlayerCache );
            continue;
        }

        let level = get_log_level(&log);

        let log = log.strip_prefix("NO LOG FILE! - ").unwrap_or(&log);
        if command_status.waiting {
            for i in 0..(log.chars().count() / 1500 + 1) {
                let result_tmp = log.chars().skip(i * 1500).take(1500).collect::<String>();
                let result: json::JsonValue = object! {
                    "command" => command_status.command.clone(),
                    "result_message" => result_tmp,
                    "count" => i,
                    "end" => i == log.chars().count() / 1500,
                };
                execute_command(
                    &child_stdin,
                    format!(
                        "scriptevent {} {} ",
                        command_status.scriptevent,
                        result.dump()
                    ),
                );
            }
            command_status.waiting = false;
        }
        let _ = stdout.write(format!("{}{}{}\n", level.to_color(), log, Color::Reset).as_bytes());

        custom_handler(log, &child_stdin);
    }
}

fn execute_command(child_stdin: &Sender<String>, command: String) {
    child_stdin.send(format!("{}\n", command)).unwrap();
}

fn execute_shell_command(command: &str, args: Vec<String>) -> Result<String, std::io::Error> {
    let output = Command::new(command).args(args).output();
    match output {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            return Ok(stdout.to_string());
        }
        Err(e) => return Err(e),
    }
}

fn build_command(os: &str, cwd: &str, executable_name: &str) -> Command {
    if os != "linux" && os != "windows" {
        panic!("Unsupported platform: {}", os);
    }

    let mut command = Command::new(Path::new(cwd).join(executable_name));

    command
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped());

    if os == "linux" {
        command.env("LD_LIBRARY_PATH", ".");
    }

    command
}

fn main() {
    let os = env::consts::OS;
    let cwd = env::args().nth(1).unwrap_or(".".to_string());
    let executable_name = env::args().nth(2).unwrap_or("bedrock_server".to_string());

    let mut child = build_command(os, &cwd, &executable_name)
        .spawn()
        .expect("Failed to spawn process");

    let child_stdin = child.stdin.take().expect("Failed to get stdin");
    let stdout = child.stdout.expect("Failed to get stdout");

    let (tx, rx) = channel::<String>();
    let tx2 = tx.clone();

    thread::spawn(move || handle_child_stdin(rx, child_stdin));
    thread::spawn(move || handle_stdin(tx));

    let mut command_status = CommandStatus {
        waiting: false,
        command: "".to_string(),
        scriptevent: "".to_string(),
    };
    handle_child_stdout(tx2, stdout, &mut command_status);
}

struct CommandStatus {
    waiting: bool,
    command: String,
    scriptevent: String,
}
