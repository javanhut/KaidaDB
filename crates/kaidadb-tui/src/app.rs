use crate::client::{self, AuthClient, MediaMetadata};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::time::Instant;
use tokio::sync::{mpsc, watch};

pub enum UploadEvent {
    Started { key: String, total_bytes: u64 },
    ChunkSent { bytes_sent: u64 },
    FileCompleted,
    FileFailed { key: String, error: String },
    Finished,
}

#[derive(Debug, Clone, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    PathBrowser,
    NewDirInput,
    StoreKey,
    FileBrowser,
    BrowserFilter,
    UploadConfirm,
    ClearMarksConfirm,
    DeleteConfirm,
    DeleteDirConfirm,
    Detail,
    RenameInput,
    MkdirInput,
    Uploading,
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
    pub server_pass: Option<String>,
    pub client: Option<AuthClient>,
    pub connected: bool,

    // Media list
    pub items: Vec<MediaMetadata>,
    pub filtered_items: Vec<usize>,
    pub selected: usize,
    pub list_scroll_offset: usize,

    // Directory browsing in main view
    pub browse_prefix: String,
    pub browse_entries: Vec<BrowseEntry>,
    // Remembered cursor position per browse prefix
    pub browse_pos: HashMap<String, usize>,

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
    pub browser_all_entries: Vec<FileEntry>,
    pub browser_entries: Vec<FileEntry>,
    pub browser_selected: usize,
    pub browser_scroll_offset: usize,
    pub browser_marked: BTreeSet<PathBuf>,
    pub browser_filter: String,
    // Remembered cursor position per local directory
    pub browser_pos: HashMap<PathBuf, usize>,

    // Rename/move
    pub rename_input: String,
    pub rename_cursor: usize,
    pub rename_original_key: String,
    pub rename_is_dir: bool,

    // Mkdir
    pub mkdir_input: String,
    pub mkdir_cursor: usize,

    // Pending upload awaiting confirmation
    pub pending_upload: Vec<(PathBuf, String)>,
    pub pending_upload_bytes: u64,
    pub pending_upload_label: String,

    // Background upload worker
    pub upload_total: usize,
    pub upload_current: usize,
    pub upload_successes: usize,
    pub upload_errors: Vec<String>,
    pub uploading: bool,
    pub upload_current_key: String,
    pub upload_current_bytes_sent: u64,
    pub upload_current_bytes_total: u64,
    pub upload_total_bytes: u64,
    pub upload_done_bytes: u64,
    pub upload_start: Option<Instant>,
    pub upload_rx: Option<mpsc::UnboundedReceiver<UploadEvent>>,
    pub upload_cancel: Option<watch::Sender<bool>>,
    pub needs_refresh_after_upload: bool,

    // Detail view
    pub detail_item: Option<MediaMetadata>,
    pub detail_scroll: usize,

    // Health
    pub health_status: String,
    pub server_version: String,

    // View options
    pub show_hidden_files: bool,
}

impl App {
    pub fn new(addr: String, server_pass: Option<String>) -> Self {
        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/"));
        // Start the file browser in the current working directory, falling back to $HOME.
        let start_dir = std::env::current_dir().unwrap_or(home);

        Self {
            addr,
            server_pass,
            client: None,
            connected: false,
            items: Vec::new(),
            filtered_items: Vec::new(),
            selected: 0,
            list_scroll_offset: 0,
            browse_prefix: String::new(),
            browse_entries: Vec::new(),
            browse_pos: HashMap::new(),
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
            browser_dir: start_dir,
            browser_all_entries: Vec::new(),
            browser_entries: Vec::new(),
            browser_selected: 0,
            browser_scroll_offset: 0,
            browser_marked: BTreeSet::new(),
            browser_filter: String::new(),
            browser_pos: HashMap::new(),
            rename_input: String::new(),
            rename_cursor: 0,
            rename_original_key: String::new(),
            rename_is_dir: false,
            mkdir_input: String::new(),
            mkdir_cursor: 0,
            pending_upload: Vec::new(),
            pending_upload_bytes: 0,
            pending_upload_label: String::new(),
            upload_total: 0,
            upload_current: 0,
            upload_successes: 0,
            upload_errors: Vec::new(),
            uploading: false,
            upload_current_key: String::new(),
            upload_current_bytes_sent: 0,
            upload_current_bytes_total: 0,
            upload_total_bytes: 0,
            upload_done_bytes: 0,
            upload_start: None,
            upload_rx: None,
            upload_cancel: None,
            needs_refresh_after_upload: false,
            detail_item: None,
            detail_scroll: 0,
            health_status: "unknown".into(),
            server_version: String::new(),
            show_hidden_files: false,
        }
    }

    pub async fn connect(&mut self) {
        match client::connect(&self.addr, self.server_pass.clone()).await {
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
                let dir_name = &suffix[..slash_pos];
                if !self.show_hidden_files && dir_name.starts_with('.') {
                    continue;
                }
                dirs.insert(dir_name.to_string());
            } else if suffix != ".kaidadb_dir" {
                if !self.show_hidden_files && suffix.starts_with('.') {
                    continue;
                }
                files.push(BrowseEntry {
                    name: suffix.to_string(),
                    is_dir: false,
                    full_key: Some(item.key.clone()),
                    item_count: 0,
                    size: item.total_size,
                });
            } else if self.show_hidden_files {
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
                // Remember where we were so going back lands on this directory.
                self.browse_pos.insert(self.browse_prefix.clone(), self.selected);
                self.browse_prefix = format!("{}{}/", self.browse_prefix, entry.name);
                self.selected = self
                    .browse_pos
                    .get(&self.browse_prefix)
                    .copied()
                    .unwrap_or(0);
                self.rebuild_browse_entries();
            }
        }
    }

    pub fn browse_up(&mut self) {
        if self.browse_prefix.is_empty() {
            return;
        }
        self.browse_pos.insert(self.browse_prefix.clone(), self.selected);
        let trimmed = self.browse_prefix.trim_end_matches('/');
        if let Some(pos) = trimmed.rfind('/') {
            self.browse_prefix = trimmed[..=pos].to_string();
        } else {
            self.browse_prefix.clear();
        }
        // Restore the cursor to the directory we just came out of.
        self.selected = self
            .browse_pos
            .get(&self.browse_prefix)
            .copied()
            .unwrap_or(0);
        self.rebuild_browse_entries();
    }

    pub fn view_detail(&mut self) {
        if let Some(entry) = self.browse_entries.get(self.selected) {
            if entry.is_dir {
                self.browse_into();
            } else if let Some(item) = self.selected_item() {
                self.detail_item = Some(item.clone());
                self.detail_scroll = 0;
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

    /// Move the cursor down in whichever list is active (mouse wheel / scroll).
    pub fn scroll_down(&mut self) {
        match self.input_mode {
            InputMode::Normal => self.next(),
            InputMode::PathBrowser => self.path_next(),
            InputMode::FileBrowser | InputMode::BrowserFilter => self.browser_next(),
            InputMode::Detail => self.detail_scroll_down(),
            _ => {}
        }
    }

    /// Move the cursor up in whichever list is active (mouse wheel / scroll).
    pub fn scroll_up(&mut self) {
        match self.input_mode {
            InputMode::Normal => self.previous(),
            InputMode::PathBrowser => self.path_previous(),
            InputMode::FileBrowser | InputMode::BrowserFilter => self.browser_previous(),
            InputMode::Detail => self.detail_scroll_up(),
            _ => {}
        }
    }

    /// Scroll the detail view content down one line.
    pub fn detail_scroll_down(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_add(1);
    }

    /// Scroll the detail view content up one line.
    pub fn detail_scroll_up(&mut self) {
        self.detail_scroll = self.detail_scroll.saturating_sub(1);
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
        // Pick *what* to upload first; the destination (path_prefix) defaults to
        // wherever we're browsing and can be changed via Tab (path browser).
        self.store_key_input.clear();
        self.store_key_cursor = 0;
        self.path_prefix = self.browse_prefix.clone();
        self.browser_filter.clear();
        self.input_mode = InputMode::FileBrowser;
        self.load_browser_dir();
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
                if !self.show_hidden_files && name.starts_with('.') {
                    continue;
                }
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

        self.browser_all_entries = entries;
        self.apply_browser_filter();
        // Restore remembered cursor position for this directory.
        let remembered = self
            .browser_pos
            .get(&self.browser_dir)
            .copied()
            .unwrap_or(0);
        self.browser_selected = remembered.min(self.browser_entries.len().saturating_sub(1));
        self.browser_scroll_offset = 0;
    }

    /// Rebuild `browser_entries` from `browser_all_entries`, applying the typeahead filter.
    pub fn apply_browser_filter(&mut self) {
        if self.browser_filter.is_empty() {
            self.browser_entries = self.browser_all_entries.clone();
        } else {
            let q = self.browser_filter.to_lowercase();
            self.browser_entries = self
                .browser_all_entries
                .iter()
                .filter(|e| e.name.to_lowercase().contains(&q))
                .cloned()
                .collect();
        }
        if self.browser_selected >= self.browser_entries.len() {
            self.browser_selected = self.browser_entries.len().saturating_sub(1);
        }
    }

    pub fn browser_filter_push(&mut self, c: char) {
        self.browser_filter.push(c);
        self.browser_selected = 0;
        self.apply_browser_filter();
    }

    pub fn browser_filter_backspace(&mut self) {
        self.browser_filter.pop();
        self.browser_selected = 0;
        self.apply_browser_filter();
    }

    pub fn browser_filter_clear(&mut self) {
        self.browser_filter.clear();
        self.browser_selected = 0;
        self.apply_browser_filter();
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
                let target = entry.path.clone();
                self.browser_pos
                    .insert(self.browser_dir.clone(), self.browser_selected);
                self.browser_filter.clear();
                self.browser_dir = target;
                self.load_browser_dir();
            }
            // File selection is handled by the caller (main.rs) via browser_select_file
        }
    }

    pub fn browser_go_up(&mut self) {
        if let Some(parent) = self.browser_dir.parent() {
            self.browser_pos
                .insert(self.browser_dir.clone(), self.browser_selected);
            self.browser_filter.clear();
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

    pub fn browser_is_marked(&self, path: &Path) -> bool {
        self.browser_marked.contains(path)
    }

    pub fn browser_toggle_mark(&mut self) {
        let Some(entry) = self.browser_entries.get(self.browser_selected) else {
            return;
        };
        // Both files and directories can be marked; directories upload recursively.
        let path = entry.path.clone();
        if !self.browser_marked.remove(&path) {
            self.browser_marked.insert(path);
        }
        self.browser_next();
    }

    pub fn browser_clear_marks(&mut self) {
        self.browser_marked.clear();
    }

    /// Auto-suggest a key from the path_prefix + filename
    pub fn suggest_key_from_path(&mut self, path: &Path) {
        if let Some(filename) = path.file_name() {
            let name = filename.to_string_lossy();
            self.store_key_input = format!("{}{}", self.path_prefix, name);
            self.store_key_cursor = self.store_key_input.len();
        }
    }

    pub fn execute_store_file(&mut self, file_path: &Path) {
        let key = self.store_key_input.clone();

        if key.is_empty() {
            self.status_message = "Key is required".into();
            self.input_mode = InputMode::Normal;
            return;
        }

        self.store_key_input.clear();
        self.store_key_cursor = 0;

        let pending = vec![(file_path.to_path_buf(), key)];
        self.begin_upload(pending);
    }

    /// Map a directory's files to (path, key) pairs preserving the tree below `base`.
    /// `base` is the prefix stripped from each file path; the remainder is appended
    /// to `path_prefix`. Passing the directory's parent as `base` keeps the directory
    /// name in the key.
    fn map_dir_files(&self, dir_path: &Path, base: &Path) -> Vec<(PathBuf, String)> {
        let mut pending = Vec::new();
        for file_path in collect_files_recursive(dir_path) {
            let relative = file_path.strip_prefix(base).unwrap_or(file_path.as_path());
            let rel_str = relative.to_string_lossy().replace('\\', "/");
            let key = format!("{}{}", self.path_prefix, rel_str);
            pending.push((file_path, key));
        }
        pending
    }

    /// Build the confirmation preview for uploading an entire directory.
    pub fn prepare_directory_upload(&mut self, dir_path: &Path) {
        let pending = self.map_dir_files(dir_path, dir_path);
        if pending.is_empty() {
            self.status_message = "Directory is empty or unreadable".into();
            return;
        }
        let label = format!("directory {}", dir_path.display());
        self.stage_pending_upload(pending, label);
    }

    /// Build the confirmation preview for uploading marked files and directories.
    /// Directories preserve their structure (their own name is kept in the key).
    pub fn prepare_marked_upload(&mut self) {
        if self.browser_marked.is_empty() {
            self.status_message = "No files marked".into();
            return;
        }

        let marked: Vec<PathBuf> = self.browser_marked.iter().cloned().collect();
        let mut pending: Vec<(PathBuf, String)> = Vec::new();
        for path in marked {
            if path.is_dir() {
                let base = path.parent().unwrap_or(path.as_path());
                pending.extend(self.map_dir_files(&path, base));
            } else {
                let filename = path
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                if filename.is_empty() {
                    continue;
                }
                let key = format!("{}{}", self.path_prefix, filename);
                pending.push((path, key));
            }
        }

        if pending.is_empty() {
            self.status_message = "Nothing to upload".into();
            return;
        }
        let label = format!("{} marked item(s)", self.browser_marked.len());
        self.stage_pending_upload(pending, label);
    }

    /// Stash a prepared upload and switch to the confirmation view.
    fn stage_pending_upload(&mut self, pending: Vec<(PathBuf, String)>, label: String) {
        let total_bytes: u64 = pending
            .iter()
            .map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        self.pending_upload = pending;
        self.pending_upload_bytes = total_bytes;
        self.pending_upload_label = label;
        self.input_mode = InputMode::UploadConfirm;
    }

    pub fn confirm_pending_upload(&mut self) {
        let pending = std::mem::take(&mut self.pending_upload);
        self.browser_marked.clear();
        if pending.is_empty() {
            self.input_mode = InputMode::Normal;
            return;
        }
        let total = pending.len();
        self.begin_upload(pending);
        self.status_message = format!("Starting upload of {} files...", total);
    }

    pub fn cancel_pending_upload(&mut self) {
        self.pending_upload.clear();
        self.pending_upload_bytes = 0;
        self.pending_upload_label.clear();
        self.input_mode = InputMode::FileBrowser;
    }

    fn begin_upload(&mut self, pending: Vec<(PathBuf, String)>) {
        self.upload_total = pending.len();
        self.upload_current = 0;
        self.upload_successes = 0;
        self.upload_errors = Vec::new();
        self.upload_total_bytes = pending
            .iter()
            .map(|(p, _)| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        self.upload_done_bytes = 0;
        self.upload_start = Some(Instant::now());

        let client = match self.client.clone() {
            Some(c) => c,
            None => {
                self.uploading = false;
                self.input_mode = InputMode::Normal;
                self.status_message = "Not connected to server".into();
                return;
            }
        };

        let (tx, rx) = mpsc::unbounded_channel();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        tokio::spawn(run_upload_worker(client, pending, tx, cancel_rx));

        self.upload_rx = Some(rx);
        self.upload_cancel = Some(cancel_tx);
        self.uploading = true;
        self.input_mode = InputMode::Uploading;
    }

    pub fn drain_upload_events(&mut self) {
        let mut finished = false;
        if let Some(rx) = self.upload_rx.as_mut() {
            loop {
                match rx.try_recv() {
                    Ok(UploadEvent::Started { key, total_bytes }) => {
                        self.upload_current += 1;
                        self.upload_current_key = key.clone();
                        self.upload_current_bytes_sent = 0;
                        self.upload_current_bytes_total = total_bytes;
                        self.status_message = format!(
                            "Uploading {}/{}: {}",
                            self.upload_current, self.upload_total, key
                        );
                    }
                    Ok(UploadEvent::ChunkSent { bytes_sent }) => {
                        self.upload_current_bytes_sent = bytes_sent;
                    }
                    Ok(UploadEvent::FileCompleted) => {
                        self.upload_successes += 1;
                        self.upload_current_bytes_sent = self.upload_current_bytes_total;
                        self.upload_done_bytes += self.upload_current_bytes_total;
                    }
                    Ok(UploadEvent::FileFailed { key, error }) => {
                        self.upload_errors.push(format!("{}: {}", key, error));
                        self.upload_done_bytes += self.upload_current_bytes_total;
                    }
                    Ok(UploadEvent::Finished) => {
                        finished = true;
                        break;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => break,
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        finished = true;
                        break;
                    }
                }
            }
        }

        if finished {
            self.upload_rx = None;
            self.upload_cancel = None;
            self.uploading = false;
            let error_count = self.upload_errors.len();
            let done = self.upload_successes + error_count;
            self.status_message = if done < self.upload_total {
                format!(
                    "Upload cancelled ({}/{} completed, {} errors)",
                    self.upload_successes, self.upload_total, error_count
                )
            } else if error_count == 0 {
                format!("Uploaded {} files", self.upload_successes)
            } else {
                format!(
                    "Uploaded {}/{} files ({} errors)",
                    self.upload_successes, self.upload_total, error_count
                )
            };
            self.input_mode = InputMode::Normal;
            self.needs_refresh_after_upload = true;
        }
    }

    pub fn cancel_upload(&mut self) {
        if let Some(tx) = self.upload_cancel.as_ref() {
            let _ = tx.send(true);
        }
    }

    /// Total bytes sent across the whole batch so far.
    pub fn upload_overall_sent(&self) -> u64 {
        self.upload_done_bytes + self.upload_current_bytes_sent
    }

    /// Whole-batch completion percentage (0-100).
    pub fn upload_overall_pct(&self) -> usize {
        if self.upload_total_bytes == 0 {
            return 0;
        }
        ((self.upload_overall_sent().min(self.upload_total_bytes) * 100) / self.upload_total_bytes)
            as usize
    }

    /// Average upload speed in bytes/sec since the batch started.
    pub fn upload_speed_bps(&self) -> f64 {
        match self.upload_start {
            Some(start) => {
                let elapsed = start.elapsed().as_secs_f64();
                if elapsed > 0.0 {
                    self.upload_overall_sent() as f64 / elapsed
                } else {
                    0.0
                }
            }
            None => 0.0,
        }
    }

    /// Estimated seconds remaining, or None if not enough data yet.
    pub fn upload_eta_secs(&self) -> Option<u64> {
        let speed = self.upload_speed_bps();
        if speed <= 0.0 || self.upload_total_bytes == 0 {
            return None;
        }
        let remaining = self.upload_total_bytes.saturating_sub(self.upload_overall_sent());
        Some((remaining as f64 / speed) as u64)
    }

    pub fn enter_delete_confirm(&mut self) {
        if let Some(entry) = self.selected_browse_entry() {
            if entry.is_dir {
                self.input_mode = InputMode::DeleteDirConfirm;
                return;
            }
        }
        if self.selected_item().is_some() {
            self.input_mode = InputMode::DeleteConfirm;
        }
    }

    pub fn toggle_hidden_files(&mut self) {
        self.show_hidden_files = !self.show_hidden_files;
        self.rebuild_browse_entries();
        self.load_browser_dir();
        self.status_message = format!(
            "Hidden files: {}",
            if self.show_hidden_files { "on" } else { "off" }
        );
    }

    pub fn selected_dir_prefix(&self) -> Option<String> {
        let entry = self.selected_browse_entry()?;
        if !entry.is_dir {
            return None;
        }
        Some(format!("{}{}/", self.browse_prefix, entry.name))
    }

    // --- Rename / Move ---

    pub fn enter_rename_mode(&mut self) {
        if let Some(entry) = self.selected_browse_entry() {
            if entry.is_dir {
                // Directory rename: original key is the full prefix (without trailing slash)
                let dir_path = format!("{}{}", self.browse_prefix, entry.name);
                self.rename_original_key = dir_path.clone();
                self.rename_input = dir_path;
                self.rename_cursor = self.rename_input.len();
                self.rename_is_dir = true;
                self.input_mode = InputMode::RenameInput;
                return;
            }
        }
        let key = self.selected_item().map(|i| i.key.clone());
        if let Some(key) = key {
            self.rename_original_key = key.clone();
            self.rename_input = key;
            self.rename_cursor = self.rename_input.len();
            self.rename_is_dir = false;
            self.input_mode = InputMode::RenameInput;
        }
    }

    pub fn rename_insert_char(&mut self, c: char) {
        self.rename_input.insert(self.rename_cursor, c);
        self.rename_cursor += c.len_utf8();
    }

    pub fn rename_backspace(&mut self) {
        if self.rename_cursor > 0 {
            let prev = self.rename_input[..self.rename_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.rename_input.drain(prev..self.rename_cursor);
            self.rename_cursor = prev;
        }
    }

    pub fn rename_delete(&mut self) {
        if self.rename_cursor < self.rename_input.len() {
            let next = self.rename_input[self.rename_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.rename_cursor + i)
                .unwrap_or(self.rename_input.len());
            self.rename_input.drain(self.rename_cursor..next);
        }
    }

    pub fn rename_move_left(&mut self) {
        if self.rename_cursor > 0 {
            self.rename_cursor = self.rename_input[..self.rename_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn rename_move_right(&mut self) {
        if self.rename_cursor < self.rename_input.len() {
            self.rename_cursor = self.rename_input[self.rename_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.rename_cursor + i)
                .unwrap_or(self.rename_input.len());
        }
    }

    pub fn rename_home(&mut self) {
        self.rename_cursor = 0;
    }

    pub fn rename_end(&mut self) {
        self.rename_cursor = self.rename_input.len();
    }

    pub async fn execute_rename(&mut self) {
        let new_path = self.rename_input.trim().to_string();
        let old_path = self.rename_original_key.clone();
        let is_dir = self.rename_is_dir;

        if new_path.is_empty() {
            self.status_message = "Key cannot be empty".into();
            return;
        }
        if new_path == old_path {
            self.status_message = "Name unchanged".into();
            self.input_mode = InputMode::Normal;
            return;
        }

        if is_dir {
            // Directory rename: find all keys under old_path/ and rename each
            let old_prefix = format!("{}/", old_path);
            let new_prefix = format!("{}/", new_path);
            let keys_to_rename: Vec<String> = self
                .items
                .iter()
                .filter(|i| i.key.starts_with(&old_prefix))
                .map(|i| i.key.clone())
                .collect();

            if keys_to_rename.is_empty() {
                self.status_message = format!("No items found under '{}'", old_prefix);
                self.input_mode = InputMode::Normal;
                return;
            }

            let mut success = 0usize;
            let mut failed = 0usize;

            if let Some(ref mut client) = self.client {
                for old_key in &keys_to_rename {
                    let new_key = format!("{}{}", new_prefix, &old_key[old_prefix.len()..]);
                    match client
                        .rename_media(client::RenameMediaRequest {
                            from_key: old_key.clone(),
                            to_key: new_key,
                        })
                        .await
                    {
                        Ok(_) => success += 1,
                        Err(_) => failed += 1,
                    }
                }
            }

            if failed == 0 {
                self.status_message = format!(
                    "Renamed directory '{}' -> '{}' ({} items)",
                    old_path, new_path, success
                );
            } else {
                self.status_message = format!(
                    "Renamed {success} items, {failed} failed ('{old_path}' -> '{new_path}')"
                );
            }
            self.refresh_media_list().await;
        } else {
            // Single file rename
            if let Some(ref mut client) = self.client {
                match client
                    .rename_media(client::RenameMediaRequest {
                        from_key: old_path.clone(),
                        to_key: new_path.clone(),
                    })
                    .await
                {
                    Ok(_resp) => {
                        self.status_message = format!("Renamed '{}' -> '{}'", old_path, new_path);
                        self.refresh_media_list().await;
                    }
                    Err(e) => {
                        self.status_message = format!("Rename failed: {e}");
                    }
                }
            }
        }

        self.rename_input.clear();
        self.rename_cursor = 0;
        self.rename_original_key.clear();
        self.rename_is_dir = false;
        self.input_mode = InputMode::Normal;
    }

    // --- Mkdir ---

    pub fn enter_mkdir_mode(&mut self) {
        self.mkdir_input.clear();
        self.mkdir_cursor = 0;
        self.input_mode = InputMode::MkdirInput;
    }

    pub fn mkdir_insert_char(&mut self, c: char) {
        self.mkdir_input.insert(self.mkdir_cursor, c);
        self.mkdir_cursor += c.len_utf8();
    }

    pub fn mkdir_backspace(&mut self) {
        if self.mkdir_cursor > 0 {
            let prev = self.mkdir_input[..self.mkdir_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            self.mkdir_input.drain(prev..self.mkdir_cursor);
            self.mkdir_cursor = prev;
        }
    }

    pub fn mkdir_delete(&mut self) {
        if self.mkdir_cursor < self.mkdir_input.len() {
            let next = self.mkdir_input[self.mkdir_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.mkdir_cursor + i)
                .unwrap_or(self.mkdir_input.len());
            self.mkdir_input.drain(self.mkdir_cursor..next);
        }
    }

    pub fn mkdir_move_left(&mut self) {
        if self.mkdir_cursor > 0 {
            self.mkdir_cursor = self.mkdir_input[..self.mkdir_cursor]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
        }
    }

    pub fn mkdir_move_right(&mut self) {
        if self.mkdir_cursor < self.mkdir_input.len() {
            self.mkdir_cursor = self.mkdir_input[self.mkdir_cursor..]
                .char_indices()
                .nth(1)
                .map(|(i, _)| self.mkdir_cursor + i)
                .unwrap_or(self.mkdir_input.len());
        }
    }

    pub async fn execute_mkdir(&mut self) {
        let dir_name = self.mkdir_input.trim().to_string();
        if dir_name.is_empty() {
            self.input_mode = InputMode::Normal;
            return;
        }

        // Store a zero-byte marker to make the directory persist
        let marker_key = format!("{}{}/{}", self.browse_prefix, dir_name, ".kaidadb_dir");

        if let Some(ref mut client) = self.client {
            let header = client::StoreMediaRequest {
                request: Some(client::store_media_request::Request::Header(
                    client::StoreMediaHeader {
                        key: marker_key.clone(),
                        content_type: "application/x-directory".into(),
                        metadata: Default::default(),
                    },
                )),
            };

            match client.store_media(tokio_stream::iter(vec![header])).await {
                Ok(_) => {
                    self.status_message = format!("Created directory '{}{}'", self.browse_prefix, dir_name);
                    self.refresh_media_list().await;
                }
                Err(e) => {
                    self.status_message = format!("Mkdir failed: {e}");
                }
            }
        }

        self.mkdir_input.clear();
        self.mkdir_cursor = 0;
        self.input_mode = InputMode::Normal;
    }

    // --- Delete ---

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

    pub async fn execute_delete_directory(&mut self) {
        let prefix = match self.selected_dir_prefix() {
            Some(p) => p,
            None => return,
        };

        let keys: Vec<String> = self
            .items
            .iter()
            .filter(|i| i.key.starts_with(&prefix))
            .map(|i| i.key.clone())
            .collect();

        if keys.is_empty() {
            self.status_message = format!("Directory '{}' is already empty", prefix);
            self.refresh_media_list().await;
            return;
        }

        let total = keys.len();
        let mut deleted = 0usize;
        let mut errors = 0usize;

        if let Some(ref mut client) = self.client {
            for key in &keys {
                match client
                    .delete_media(client::DeleteMediaRequest { key: key.clone() })
                    .await
                {
                    Ok(resp) => {
                        if resp.into_inner().deleted {
                            deleted += 1;
                        }
                    }
                    Err(_) => {
                        errors += 1;
                    }
                }
            }
        }

        self.status_message = format!(
            "Force-deleted '{}' ({}/{} deleted, {} errors)",
            prefix, deleted, total, errors
        );
        self.refresh_media_list().await;
    }
}

fn collect_files_recursive(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                files.extend(collect_files_recursive(&path));
            } else if path.is_file() {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

async fn run_upload_worker(
    mut client: AuthClient,
    pending: Vec<(PathBuf, String)>,
    tx: mpsc::UnboundedSender<UploadEvent>,
    mut cancel_rx: watch::Receiver<bool>,
) {
    use tokio_stream::StreamExt;

    for (file_path, key) in pending {
        if *cancel_rx.borrow() {
            break;
        }

        let data = match tokio::fs::read(&file_path).await {
            Ok(d) => d,
            Err(e) => {
                if tx
                    .send(UploadEvent::Started {
                        key: key.clone(),
                        total_bytes: 0,
                    })
                    .is_err()
                {
                    return;
                }
                let _ = tx.send(UploadEvent::FileFailed {
                    key,
                    error: e.to_string(),
                });
                continue;
            }
        };

        let total_bytes = data.len() as u64;
        if tx
            .send(UploadEvent::Started {
                key: key.clone(),
                total_bytes,
            })
            .is_err()
        {
            return;
        }

        let ct = client::guess_content_type(&file_path.to_string_lossy()).to_string();

        let header = client::StoreMediaRequest {
            request: Some(client::store_media_request::Request::Header(
                client::StoreMediaHeader {
                    key: key.clone(),
                    content_type: ct,
                    metadata: Default::default(),
                },
            )),
        };
        let chunk_size = 2 * 1024 * 1024usize;
        let mut items: Vec<(u64, client::StoreMediaRequest)> = Vec::new();
        items.push((0, header));
        let mut running = 0u64;
        for chunk in data.chunks(chunk_size) {
            running += chunk.len() as u64;
            items.push((
                running,
                client::StoreMediaRequest {
                    request: Some(client::store_media_request::Request::ChunkData(
                        chunk.to_vec(),
                    )),
                },
            ));
        }

        let tx_progress = tx.clone();
        let stream = tokio_stream::iter(items).map(move |(bytes_sent, msg)| {
            let _ = tx_progress.send(UploadEvent::ChunkSent { bytes_sent });
            msg
        });

        let call = client.store_media(stream);
        tokio::select! {
            res = cancel_rx.changed() => {
                let _ = res;
                break;
            }
            res = call => {
                match res {
                    Ok(_) => { let _ = tx.send(UploadEvent::FileCompleted); }
                    Err(e) => {
                        let _ = tx.send(UploadEvent::FileFailed {
                            key,
                            error: e.to_string(),
                        });
                    }
                }
            }
        }
    }

    let _ = tx.send(UploadEvent::Finished);
}
