use crate::client::{self, KaidaDbClient, MediaMetadata};
use tonic::transport::Channel;

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    StoreKey,
    StorePath,
    DeleteConfirm,
    Detail,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Panel {
    List,
    Detail,
}

pub struct App {
    pub addr: String,
    pub client: Option<KaidaDbClient<Channel>>,
    pub connected: bool,

    // Media list
    pub items: Vec<MediaMetadata>,
    pub filtered_items: Vec<usize>, // indices into items
    pub selected: usize,

    // UI state
    pub input_mode: InputMode,
    pub active_panel: Panel,
    pub status_message: String,

    // Search
    pub search_input: String,
    pub search_query: String,

    // Store dialog
    pub store_key_input: String,
    pub store_path_input: String,

    // Detail view
    pub detail_item: Option<MediaMetadata>,

    // Health
    pub health_status: String,
    pub server_version: String,
}

impl App {
    pub fn new(addr: String) -> Self {
        Self {
            addr,
            client: None,
            connected: false,
            items: Vec::new(),
            filtered_items: Vec::new(),
            selected: 0,
            input_mode: InputMode::Normal,
            active_panel: Panel::List,
            status_message: "Connecting...".into(),
            search_input: String::new(),
            search_query: String::new(),
            store_key_input: String::new(),
            store_path_input: String::new(),
            detail_item: None,
            health_status: "unknown".into(),
            server_version: String::new(),
        }
    }

    pub async fn connect(&mut self) {
        match client::connect(&self.addr).await {
            Ok(c) => {
                self.client = Some(c);
                self.connected = true;
                self.status_message = format!("Connected to {}", self.addr);
            }
            Err(e) => {
                self.connected = false;
                self.status_message = format!("Connection failed: {e}");
            }
        }
    }

    pub async fn check_health(&mut self) {
        if let Some(ref mut client) = self.client {
            match client
                .health_check(client::HealthCheckRequest {})
                .await
            {
                Ok(resp) => {
                    let h = resp.into_inner();
                    self.health_status = h.status;
                    self.server_version = h.version;
                }
                Err(_) => {
                    self.health_status = "unreachable".into();
                }
            }
        }
    }

    pub async fn refresh_media_list(&mut self) {
        if let Some(ref mut client) = self.client {
            match client
                .list_media(client::ListMediaRequest {
                    prefix: String::new(),
                    limit: 1000,
                    cursor: String::new(),
                })
                .await
            {
                Ok(resp) => {
                    self.items = resp.into_inner().items;
                    self.apply_filter();
                    self.status_message = format!("{} media items loaded", self.items.len());
                }
                Err(e) => {
                    self.status_message = format!("Failed to list: {e}");
                }
            }
        }
    }

    fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_items = (0..self.items.len()).collect();
        } else {
            let q = self.search_query.to_lowercase();
            self.filtered_items = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, item)| {
                    item.key.to_lowercase().contains(&q)
                        || item.content_type.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.selected >= self.filtered_items.len() {
            self.selected = self.filtered_items.len().saturating_sub(1);
        }
    }

    pub fn selected_item(&self) -> Option<&MediaMetadata> {
        self.filtered_items
            .get(self.selected)
            .and_then(|&idx| self.items.get(idx))
    }

    pub fn next(&mut self) {
        if !self.filtered_items.is_empty() {
            self.selected = (self.selected + 1).min(self.filtered_items.len() - 1);
        }
    }

    pub fn previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        if !self.filtered_items.is_empty() {
            self.selected = self.filtered_items.len() - 1;
        }
    }

    pub fn view_detail(&mut self) {
        if let Some(item) = self.selected_item() {
            self.detail_item = Some(item.clone());
            self.input_mode = InputMode::Detail;
        }
    }

    pub fn back(&mut self) {
        match self.input_mode {
            InputMode::Detail => self.input_mode = InputMode::Normal,
            _ => {}
        }
    }

    pub fn toggle_panel(&mut self) {
        self.active_panel = match self.active_panel {
            Panel::List => Panel::Detail,
            Panel::Detail => Panel::List,
        };
    }

    pub fn enter_search_mode(&mut self) {
        self.search_input.clear();
        self.input_mode = InputMode::Search;
    }

    pub fn execute_search(&mut self) {
        self.search_query = self.search_input.clone();
        self.apply_filter();
    }

    pub fn search_next(&mut self) {
        if !self.search_query.is_empty() && !self.filtered_items.is_empty() {
            self.next();
        }
    }

    pub fn enter_store_mode(&mut self) {
        self.store_key_input.clear();
        self.store_path_input.clear();
        self.input_mode = InputMode::StoreKey;
    }

    pub async fn execute_store(&mut self) {
        let key = self.store_key_input.clone();
        let path = self.store_path_input.clone();

        if key.is_empty() || path.is_empty() {
            self.status_message = "Key and path are required".into();
            return;
        }

        let data = match tokio::fs::read(&path).await {
            Ok(d) => d,
            Err(e) => {
                self.status_message = format!("Failed to read file: {e}");
                return;
            }
        };

        let ct = client::guess_content_type(&path).to_string();

        if let Some(ref mut client) = self.client {
            let header = client::StoreMediaRequest {
                request: Some(client::store_media_request::Request::Header(
                    client::StoreMediaHeader {
                        key: key.clone(),
                        content_type: ct,
                        metadata: Default::default(),
                    },
                )),
            };

            let chunk_size = 2 * 1024 * 1024;
            let mut messages = vec![header];
            for chunk in data.chunks(chunk_size) {
                messages.push(client::StoreMediaRequest {
                    request: Some(client::store_media_request::Request::ChunkData(
                        chunk.to_vec(),
                    )),
                });
            }

            match client
                .store_media(tokio_stream::iter(messages))
                .await
            {
                Ok(resp) => {
                    let r = resp.into_inner();
                    self.status_message = format!(
                        "Stored '{}': {} bytes, {} chunks",
                        r.key, r.total_size, r.chunk_count
                    );
                    self.refresh_media_list().await;
                }
                Err(e) => {
                    self.status_message = format!("Store failed: {e}");
                }
            }
        }

        self.store_key_input.clear();
        self.store_path_input.clear();
    }

    pub fn enter_delete_confirm(&mut self) {
        if self.selected_item().is_some() {
            self.input_mode = InputMode::DeleteConfirm;
        }
    }

    pub async fn execute_delete(&mut self) {
        let key = match self.selected_item() {
            Some(item) => item.key.clone(),
            None => return,
        };

        if let Some(ref mut client) = self.client {
            match client
                .delete_media(client::DeleteMediaRequest { key: key.clone() })
                .await
            {
                Ok(resp) => {
                    if resp.into_inner().deleted {
                        self.status_message = format!("Deleted '{key}'");
                    } else {
                        self.status_message = format!("'{key}' not found");
                    }
                    self.refresh_media_list().await;
                }
                Err(e) => {
                    self.status_message = format!("Delete failed: {e}");
                }
            }
        }
    }
}
