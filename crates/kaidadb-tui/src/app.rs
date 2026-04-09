use crate::client::{self, KaidaDbClient, MediaMetadata};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use tonic::transport::Channel;

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    PathBrowser,
    NewDirInput,
    StoreKey,
    FileBrowser,
    DeleteConfirm,
    Detail,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Panel {
    List,
    Detail,
}

#[derive(Debug, Clone)]
pub struct FileEntry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    pub size: u64,
}

#[derive(Debug, Clone)]
pub struct PathEntry {
    pub name: String,
    pub is_dir: bool,
    pub item_count: usize,
}

#[derive(Debug, Clone)]
pub struct BrowseEntry {
    pub name: String,
    pub is_dir: bool,
    pub full_key: Option<String>,
    pub item_count: usize,
    pub size: u64,
}


pub struct App {
    pub addr: String,
    pub client: Option<KaidaDbClient<Channel>>,
    pub connected: bool,

    // Media list
    pub items: Vec<MediaMetadata>,
    pub filtered_items: Vec<usize>,
    pub selected: usize,

    // Directory browsing in main view
    pub browse_prefix: String,
    pub browse_entries: Vec<BrowseEntry>,

    // UI state
    pub input_mode: InputMode,
    pub active_panel: Panel,
    pub status_message: String,

    // Search
    pub search_input: String,
    pub search_query: String,

    // Store dialog
    pub store_key_input: String,
    pub store_key_cursor: usize,
    pub selected_file_path: Option<PathBuf>,

    // Path browser (virtual KaidaDB directory tree)
    pub path_prefix: String,
    pub path_entries: Vec<PathEntry>,
    pub path_selected: usize,
    pub path_scroll_offset: usize,

    // New directory input
    pub new_dir_input: String,
    pub new_dir_cursor: usize,

    // File browser
    pub browser_dir: PathBuf,
    pub browser_entries: Vec<FileEntry>,
    pub browser_selected: usize,
    pub browser_scroll_offset: usize,

    // Detail view
    pub detail_item: Option<MediaMetadata>,

    // Health
    pub health_status: String,
    pub server_version: String,
}

impl App {
    pub fn new(addr: String) -> Self {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));

        Self {
            addr,
            client: None,
            connected: false,
            items: Vec::new(),
            filtered_items: Vec::new(),
            selected: 0,
            browse_prefix: String::new(),
            browse_entries: Vec::new(),
            input_mode: InputMode::Normal,
            active_panel: Panel::List,
            status_message: "Connecting...".into(),
            search_input: String::new(),
            search_query: String::new(),
            store_key_input: String::new(),
            store_key_cursor: 0,
            selected_file_path: None,
            path_prefix: String::new(),
            path_entries: Vec::new(),
            path_selected: 0,
            path_scroll_offset: 0,
            new_dir_input: String::new(),
            new_dir_cursor: 0,
            browser_dir: home,
            browser_entries: Vec::new(),
            browser_selected: 0,
            browser_scroll_offset: 0,
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
        self.rebuild_browse_entries();
    }

    pub fn rebuild_browse_entries(&mut self) {
        let prefix = &self.browse_prefix;
        let mut dirs = BTreeSet::new();
        let mut files: Vec<BrowseEntry> = Vec::new();

        // Use filtered_items to respect search
        for &idx in &self.filtered_items {
            let item = &self.items[idx];
            if !prefix.is_empty() && !item.key.starts_with(prefix.as_str()) {
                continue;
            }
            let suffix = &item.key[prefix.len()..];
            if suffix.is_empty() {
                continue;
            }
            if let Some(slash_pos) = suffix.find('/') {
                dirs.insert(suffix[..slash_pos].to_string());
            } else {
                files.push(BrowseEntry {
                    name: suffix.to_string(),
                    is_dir: false,
                    full_key: Some(item.key.clone()),
                    item_count: 0,
                    size: item.total_size,
                });
            }
        }

        let mut entries: Vec<BrowseEntry> = Vec::new();

        for dir_name in &dirs {
            let dir_prefix = format!("{}{}/", prefix, dir_name);
            let count = self
                .filtered_items
                .iter()
                .filter(|&&idx| self.items[idx].key.starts_with(&dir_prefix))
                .count();
            entries.push(BrowseEntry {
                name: dir_name.clone(),
                is_dir: true,
                full_key: None,
                item_count: count,
                size: 0,
            });
        }

        files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        entries.extend(files);

        self.browse_entries = entries;
        if self.selected >= self.browse_entries.len() {
            self.selected = self.browse_entries.len().saturating_sub(1);
        }
    }

    pub fn selected_browse_entry(&self) -> Option<&BrowseEntry> {
        self.browse_entries.get(self.selected)
    }

    pub fn selected_item(&self) -> Option<&MediaMetadata> {
        let entry = self.browse_entries.get(self.selected)?;
        let key = entry.full_key.as_ref()?;
        self.items.iter().find(|i| &i.key == key)
    }

    pub fn next(&mut self) {
        if !self.browse_entries.is_empty() {
            self.selected = (self.selected + 1).min(self.browse_entries.len() - 1);
        }
    }

    pub fn previous(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    pub fn first(&mut self) {
        self.selected = 0;
    }

    pub fn last(&mut self) {
        if !self.browse_entries.is_empty() {
            self.selected = self.browse_entries.len() - 1;
        }
    }

    pub fn browse_into(&mut self) {
        if let Some(entry) = self.browse_entries.get(self.selected) {
            if entry.is_dir {
                self.browse_prefix = format!("{}{}/", self.browse_prefix, entry.name);
                self.selected = 0;
                self.rebuild_browse_entries();
            }
        }
    }

    pub fn browse_up(&mut self) {
        if self.browse_prefix.is_empty() {
            return;
        }
        let trimmed = self.browse_prefix.trim_end_matches('/');
        if let Some(pos) = trimmed.rfind('/') {
            self.browse_prefix = trimmed[..=pos].to_string();
        } else {
            self.browse_prefix.clear();
        }
        self.selected = 0;
        self.rebuild_browse_entries();
    }

    pub fn view_detail(&mut self) {
        if let Some(entry) = self.browse_entries.get(self.selected) {
            if entry.is_dir {
                self.browse_into();
            } else if let Some(item) = self.selected_item() {
                self.detail_item = Some(item.clone());
                self.input_mode = InputMode::Detail;
            }
        }
    }

    pub fn back(&mut self) {
        match self.input_mode {
            InputMode::Detail => {
                self.input_mode = InputMode::Normal;
            }
            InputMode::Normal => {
                self.browse_up();
            }
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

    // --- Path Browser (virtual KaidaDB directory tree) ---

    pub fn enter_store_mode(&mut self) {
        self.store_key_input.clear();
        self.store_key_cursor = 0;
        self.path_prefix.clear();
        self.input_mode = InputMode::PathBrowser;
        self.load_path_entries();
    }

    pub fn load_path_entries(&mut self) {
        let prefix = &self.path_prefix;
        let mut dirs = BTreeSet::new();
        let mut files = Vec::new();

        for item in &self.items {
            if !prefix.is_empty() && !item.key.starts_with(prefix.as_str()) {
                continue;
            }
            let suffix = &item.key[prefix.len()..];
            if suffix.is_empty() {
                continue;
            }
            if let Some(slash_pos) = suffix.find('/') {
                dirs.insert(suffix[..slash_pos].to_string());
            } else {
                files.push(suffix.to_string());
            }
        }

        let mut entries: Vec<PathEntry> = Vec::new();

        // Directories first
        for dir_name in &dirs {
            let dir_prefix = format!("{}{}/", prefix, dir_name);
            let count = self
                .items
                .iter()
                .filter(|i| i.key.starts_with(&dir_prefix))
                .count();
            entries.push(PathEntry {
                name: dir_name.clone(),
                is_dir: true,
                item_count: count,
            });
        }

        // Then files at this level
        files.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
        for name in files {
            entries.push(PathEntry {
                name,
                is_dir: false,
                item_count: 0,
            });
        }

        self.path_entries = entries;
        self.path_selected = 0;
        self.path_scroll_offset = 0;
    }

    pub fn path_next(&mut self) {
        if !self.path_entries.is_empty() {
            self.path_selected = (self.path_selected + 1).min(self.path_entries.len() - 1);
        }
    }

    pub fn path_previous(&mut self) {
        self.path_selected = self.path_selected.saturating_sub(1);
    }

    pub fn path_first(&mut self) {
        self.path_selected = 0;
    }

    pub fn path_last(&mut self) {
        if !self.path_entries.is_empty() {
            self.path_selected = self.path_entries.len() - 1;
        }
    }

    pub fn path_enter(&mut self) {
        if let Some(entry) = self.path_entries.get(self.path_selected) {
            if entry.is_dir {
                self.path_prefix = format!("{}{}/", self.path_prefix, entry.name);
                self.load_path_entries();
            }
        }
    }

    pub fn path_go_up(&mut self) {
        if self.path_prefix.is_empty() {
            return;
        }
        // Remove trailing slash, then find last slash
        let trimmed = self.path_prefix.trim_end_matches('/');
        if let Some(pos) = trimmed.rfind('/') {
            self.path_prefix = trimmed[..=pos].to_string();
        } else {
            self.path_prefix.clear();
        }
        self.load_path_entries();
    }

    pub fn path_selected_entry(&self) -> Option<&PathEntry> {
        self.path_entries.get(self.path_selected)
    }

    pub fn enter_new_dir_mode(&mut self) {
        self.new_dir_input.clear();
        self.new_dir_cursor = 0;
        self.input_mode = InputMode::NewDirInput;
    }

    pub fn new_dir_insert_char(&mut self, c: char) {
        self.new_dir_input.insert(self.new_dir_cursor, c);
        self.new_dir_cursor += c.len_utf8();
    }

    pub fn new_dir_backspace(&mut self) {
        if self.new_dir_cursor > 0 {
            let prev = self.new_dir_input[..self.new_dir_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.new_dir_input.drain(prev..self.new_dir_cursor);
            self.new_dir_cursor = prev;
        }
    }

    pub fn new_dir_delete(&mut self) {
        if self.new_dir_cursor < self.new_dir_input.len() {
            let next = self.new_dir_input[self.new_dir_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.new_dir_cursor + i)
                .unwrap_or(self.new_dir_input.len());
            self.new_dir_input.drain(self.new_dir_cursor..next);
        }
    }

    pub fn new_dir_move_left(&mut self) {
        if self.new_dir_cursor > 0 {
            self.new_dir_cursor = self.new_dir_input[..self.new_dir_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn new_dir_move_right(&mut self) {
        if self.new_dir_cursor < self.new_dir_input.len() {
            self.new_dir_cursor = self.new_dir_input[self.new_dir_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.new_dir_cursor + i)
                .unwrap_or(self.new_dir_input.len());
        }
    }

    pub fn confirm_new_dir(&mut self) {
        let dir_name = self.new_dir_input.trim().to_string();
        if dir_name.is_empty() {
            self.input_mode = InputMode::PathBrowser;
            return;
        }
        // Navigate into the new directory (it doesn't need to exist in the DB yet)
        self.path_prefix = format!("{}{}/", self.path_prefix, dir_name);
        self.load_path_entries();
        self.input_mode = InputMode::PathBrowser;
        self.status_message = format!("Created path: {}", self.path_prefix);
    }

    pub fn advance_to_file_browser(&mut self) {
        self.input_mode = InputMode::FileBrowser;
        self.load_browser_dir();
    }

    // --- Store Key / File Browser ---

    pub fn store_key_insert_char(&mut self, c: char) {
        self.store_key_input.insert(self.store_key_cursor, c);
        self.store_key_cursor += c.len_utf8();
    }

    pub fn store_key_backspace(&mut self) {
        if self.store_key_cursor > 0 {
            let prev = self.store_key_input[..self.store_key_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.store_key_input.drain(prev..self.store_key_cursor);
            self.store_key_cursor = prev;
        }
    }

    pub fn store_key_delete(&mut self) {
        if self.store_key_cursor < self.store_key_input.len() {
            let next = self.store_key_input[self.store_key_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.store_key_cursor + i)
                .unwrap_or(self.store_key_input.len());
            self.store_key_input.drain(self.store_key_cursor..next);
        }
    }

    pub fn store_key_move_left(&mut self) {
        if self.store_key_cursor > 0 {
            self.store_key_cursor = self.store_key_input[..self.store_key_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn store_key_move_right(&mut self) {
        if self.store_key_cursor < self.store_key_input.len() {
            self.store_key_cursor = self.store_key_input[self.store_key_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.store_key_cursor + i)
                .unwrap_or(self.store_key_input.len());
        }
    }

    pub fn store_key_home(&mut self) {
        self.store_key_cursor = 0;
    }

    pub fn store_key_end(&mut self) {
        self.store_key_cursor = self.store_key_input.len();
    }

    pub fn advance_to_browser(&mut self) {
        self.input_mode = InputMode::FileBrowser;
    }

    pub fn back_to_path_browser(&mut self) {
        self.input_mode = InputMode::PathBrowser;
        self.load_path_entries();
    }

    pub fn load_browser_dir(&mut self) {
        let dir = &self.browser_dir;
        let mut entries = Vec::new();

        if let Ok(read_dir) = std::fs::read_dir(dir) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                let meta = entry.metadata().ok();
                let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
                let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);

                entries.push(FileEntry {
                    name,
                    path,
                    is_dir,
                    size,
                });
            }
        }

        // Sort: directories first, then by name
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        self.browser_entries = entries;
        self.browser_selected = 0;
        self.browser_scroll_offset = 0;
    }

    pub fn browser_next(&mut self) {
        if !self.browser_entries.is_empty() {
            self.browser_selected =
                (self.browser_selected + 1).min(self.browser_entries.len() - 1);
        }
    }

    pub fn browser_previous(&mut self) {
        self.browser_selected = self.browser_selected.saturating_sub(1);
    }

    pub fn browser_page_down(&mut self, visible_rows: usize) {
        if !self.browser_entries.is_empty() {
            self.browser_selected = (self.browser_selected + visible_rows)
                .min(self.browser_entries.len() - 1);
        }
    }

    pub fn browser_page_up(&mut self, visible_rows: usize) {
        self.browser_selected = self.browser_selected.saturating_sub(visible_rows);
    }

    pub fn browser_first(&mut self) {
        self.browser_selected = 0;
    }

    pub fn browser_last(&mut self) {
        if !self.browser_entries.is_empty() {
            self.browser_selected = self.browser_entries.len() - 1;
        }
    }

    pub fn browser_enter(&mut self) {
        if let Some(entry) = self.browser_entries.get(self.browser_selected) {
            if entry.is_dir {
                self.browser_dir = entry.path.clone();
                self.load_browser_dir();
            }
            // File selection is handled by the caller (main.rs) via browser_select_file
        }
    }

    pub fn browser_go_up(&mut self) {
        if let Some(parent) = self.browser_dir.parent() {
            self.browser_dir = parent.to_path_buf();
            self.load_browser_dir();
        }
    }

    pub fn browser_selected_entry(&self) -> Option<&FileEntry> {
        self.browser_entries.get(self.browser_selected)
    }

    pub fn browser_selected_is_file(&self) -> bool {
        self.browser_selected_entry()
            .map(|e| !e.is_dir)
            .unwrap_or(false)
    }

    /// Auto-suggest a key from the path_prefix + filename
    pub fn suggest_key_from_path(&mut self, path: &Path) {
        if let Some(filename) = path.file_name() {
            let name = filename.to_string_lossy();
            self.store_key_input = format!("{}{}", self.path_prefix, name);
            self.store_key_cursor = self.store_key_input.len();
        }
    }

    pub async fn execute_store_file(&mut self, file_path: &Path) {
        let key = self.store_key_input.clone();

        if key.is_empty() {
            self.status_message = "Key is required".into();
            return;
        }

        let data = match tokio::fs::read(file_path).await {
            Ok(d) => d,
            Err(e) => {
                self.status_message = format!("Failed to read file: {e}");
                return;
            }
        };

        let ct = client::guess_content_type(&file_path.to_string_lossy()).to_string();

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

            match client.store_media(tokio_stream::iter(messages)).await {
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
        self.store_key_cursor = 0;
    }

    pub fn enter_delete_confirm(&mut self) {
        if let Some(entry) = self.selected_browse_entry() {
            if entry.is_dir {
                self.status_message = format!(
                    "Directory has {} items, delete them first (directory disappears when empty)",
                    entry.item_count
                );
                return;
            }
        }
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
