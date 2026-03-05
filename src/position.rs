use serde::{Deserialize, Serialize};
use solana_sdk::pubkey::Pubkey;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{info, warn};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Position {
    pub mint: Pubkey,
    pub bonding_curve: Pubkey,
    pub creator: Pubkey,
    pub lamports_invested: u64,
    pub token_amount: u64,
    pub entry_timestamp: i64,
    #[serde(default)]
    pub symbol: String,
    #[serde(default)]
    pub name: String,
}

pub type PositionStore = Arc<RwLock<HashMap<Pubkey, Position>>>;

pub fn new_position_store() -> PositionStore {
    Arc::new(RwLock::new(HashMap::new()))
}

fn no_save() -> bool {
    std::env::var("NO_SAVE").is_ok_and(|v| v == "true" || v == "1")
}

fn positions_path() -> PathBuf {
    let mut path = dirs::home_dir().expect("cannot determine home directory");
    path.push(".solami");
    path.push("starter");
    path.push("positions.json");
    path
}

pub fn load_positions() -> HashMap<Pubkey, Position> {
    if no_save() {
        return HashMap::new();
    }
    let path = positions_path();
    match std::fs::read_to_string(&path) {
        Ok(data) => match serde_json::from_str::<HashMap<String, Position>>(&data) {
            Ok(str_map) => {
                let map = str_map
                    .into_iter()
                    .filter_map(|(k, v)| k.parse::<Pubkey>().ok().map(|pk| (pk, v)))
                    .collect();
                info!(path = %path.display(), "loaded positions from disk");
                map
            }
            Err(e) => {
                warn!(error = %e, path = %path.display(), "failed to parse positions file, starting empty");
                HashMap::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            info!(path = %path.display(), "no positions file found, starting empty");
            HashMap::new()
        }
        Err(e) => {
            warn!(error = %e, path = %path.display(), "failed to read positions file, starting empty");
            HashMap::new()
        }
    }
}

pub fn save_positions(store: &HashMap<Pubkey, Position>) {
    if no_save() {
        return;
    }
    let path = positions_path();
    if let Some(parent) = path.parent() {
        if let Err(e) = std::fs::create_dir_all(parent) {
            warn!(error = %e, "failed to create positions directory");
            return;
        }
    }
    let str_map: HashMap<String, &Position> = store
        .iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    match serde_json::to_string_pretty(&str_map) {
        Ok(json) => {
            let tmp = path.with_extension("json.tmp");
            if let Err(e) = std::fs::write(&tmp, &json) {
                warn!(error = %e, "failed to write temp positions file");
                return;
            }
            if let Err(e) = std::fs::rename(&tmp, &path) {
                warn!(error = %e, "failed to rename positions file");
            }
        }
        Err(e) => {
            warn!(error = %e, "failed to serialize positions");
        }
    }
}
