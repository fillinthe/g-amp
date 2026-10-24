//! The Media Library window's behaviour: the source tree, lists loaded in
//! the background, the table, search, playing and queueing, and the
//! Settings forms (folders, media servers, Dropbox, Spotify, SoundCloud).

use std::collections::{BTreeSet, HashMap};

use gamp_core::catalog::Source;
use gamp_core::skin_museum as museum;
use gamp_core::sources::{Action, Item, Listing};
use gamp_ui::library_window::{self as lw, Field, Hot, Row, TreeRow, W};
use serde_json::{Value, json};

use crate::app::{App, Kind, clock};
use crate::backend::{Reply, open_in_browser};
use crate::playlist::Entry;

/// Where museum skins wait in the data folder (Keep this skin copies one to skins/).
pub const MUSEUM_DIR: &str = "museum";

/// What a tree row opens.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Toggle(String),
    Open { sid: String, node: String, title: String },
    Search { sid: String, title: String },
    /// Settings, scrolled to the section whose heading starts so.
    Settings(Option<&'static str>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Page {
    List { sid: String, node: String, title: String },
    Search { sid: String, title: String },
    Settings,
}

impl Page {
    pub fn key(&self) -> String {
        match self {
            Page::List { sid, node, .. } => format!("{sid}\u{1f}{node}"),
            Page::Search { sid, .. } => format!("{sid}\u{1f}search"),
            Page::Settings => "settings".into(),
        }
    }
}

/// Media server kinds for the "Add server" form.
const KINDS: &[(&str, &str, &str)] = &[
    ("jellyfin", "Jellyfin or Emby", "http://nas:8096"),
    ("subsonic", "Navidrome / Subsonic", "http://nas:4533"),
    ("synology", "Synology Audio Station", "QuickConnect ID, or http://diskstation:5000"),
    ("webdav", "WebDAV / Nextcloud", "https://nas:5006/music"),
    ("plex", "Plex (address and token)", "http://nas:32400"),
];

pub struct LibState {
    pub view: lw::View,
    pub hots: Vec<(Hot, lw::R)>,
    pub sources: Vec<Source>,
    pub targets: Vec<Target>,
    pub collapsed: BTreeSet<String>,
    pub page: Page,
    pub trail: Vec<Page>,
    pub cache: HashMap<String, Listing>,
    /// The rows shown, as items (filtered and sorted).
    pub items: Vec<Item>,
    pub actions: Vec<Action>,
    pub hint: Option<String>,
    pub filter: String,
    pub sort: Option<(usize, bool)>,
    pub loading: bool,
    pub fields: HashMap<String, Field>,
    pub focus: Option<String>,
    pub config: Value,
    pub kind: usize,
    pub insecure: bool,
    pub scan: Option<gamp_core::library::Status>,
    pub anchor: Option<usize>,
}

impl Default for LibState {
    fn default() -> Self {
        let mut fields = HashMap::new();
        for (id, ph, pw) in [
            ("srv_name", "Name (optional)", false),
            ("srv_url", "http://nas:8096", false),
            ("srv_user", "User name", false),
            ("srv_pass", "Password", true),
            ("srv_otp", "Two-step code (if you use one)", false),
            ("folder_path", "Or type a path", false),
            ("share", "smb://diskstation/music", false),
            ("sc_profile", "Your profile: soundcloud.com/you", false),
        ] {
            fields.insert(id.to_owned(), Field { placeholder: ph.into(), password: pw, ..Default::default() });
        }
        let view = lw::View { width: 550, height: 348, search_shown: true, search: Field::new("Search"), ..Default::default() };
        LibState {
            view,
            hots: Vec::new(),
            sources: Vec::new(),
            targets: Vec::new(),
            collapsed: BTreeSet::new(),
            page: Page::List { sid: "local".into(), node: "audio".into(), title: "Local Media › Audio".into() },
            trail: Vec::new(),
            cache: HashMap::new(),
            items: Vec::new(),
            actions: Vec::new(),
            hint: None,
            filter: String::new(),
            sort: None,
            loading: false,
            fields,
            focus: None,
            config: Value::Null,
            kind: 0,
            insecure: false,
            scan: None,
            anchor: None,
        }
    }
}

fn is_playable(i: &Item) -> bool {
    i.kind == "track"
}

impl App {
    pub fn lib_start(&mut self) {
        if let Some(b) = &self.backend {
            b.refresh_sources();
            b.config();
            b.scan_status();
            // DLNA servers answer a few seconds after the search starts.
            self.pending_refresh = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
        }
        self.lib_open(self.lib.page.clone(), true);
    }

    // ---- replies from the background ----------------------------------------

    pub fn lib_poll(&mut self) {
        let Some(b) = &self.backend else { return };
        let replies: Vec<Reply> = b.rx.try_iter().collect();
        // Called every tick: redraw only when something came in.
        if replies.is_empty() {
            return;
        }
        for r in replies {
            match r {
                Reply::Sources(s) => {
                    self.lib.sources = s;
                    self.lib_tree();
                }
                Reply::Listing { key, result } => {
                    self.lib.loading = false;
                    match result {
                        Ok(l) => {
                            let play_now = self.pending_stream && !l.items.is_empty() && l.items.iter().all(|i| i.live || i.url.is_some());
                            self.pending_stream = false;
                            let items = l.items.clone();
                            self.lib.cache.insert(key.clone(), l);
                            if self.lib.page.key() == key {
                                self.lib_show();
                            }
                            if play_now {
                                self.lib_send(items, false);
                            }
                        }
                        Err(e) => {
                            if self.lib.page.key() == key {
                                self.lib.items.clear();
                                self.lib.hint = Some(e.clone());
                                self.lib_rows();
                            }
                            self.last_error = Some(e.clone());
                            self.lib.view.status = e;
                        }
                    }
                }
                Reply::Config(c) => {
                    if let Some(k) = c["soundcloud"]["profile"].as_str() {
                        self.lib_field_default("sc_profile", k);
                    }
                    // Still looking for DLNA servers: look at the list again shortly.
                    if c["dlnaSearching"] == true && self.pending_refresh.is_none() {
                        self.pending_refresh = Some(std::time::Instant::now() + std::time::Duration::from_secs(2));
                    }
                    self.lib.config = c;
                    if self.lib.page == Page::Settings {
                        self.lib_form();
                    }
                    self.lib_tree();
                }
                Reply::Message(m) => {
                    self.lib.view.status = m.clone();
                    self.main.message = None;
                    // The settings form covers the status line: say it in the display.
                    if self.lib.page == Page::Settings && !m.is_empty() {
                        self.say(&m);
                    }
                    // Favourites and subscriptions changed: reload them.
                    self.lib.cache.retain(|k, _| !(k.ends_with("favorites") || k.ends_with("subscriptions")));
                    if let Page::List { node, .. } = &self.lib.page {
                        if node == "favorites" || node == "subscriptions" {
                            let p = self.lib.page.clone();
                            self.lib_open(p, false);
                        }
                    }
                }
                Reply::Skin(Ok(path)) => {
                    let name = gamp_core::skin_museum::name(&path.file_name().unwrap_or_default().to_string_lossy());
                    self.load_skin(Some(path));
                    self.say(&format!("SKIN: {}", name.to_uppercase()));
                }
                Reply::Skin(Err(e)) => self.say(&e.to_uppercase()),
                Reply::Scan(s) => {
                    let scanning = s.scanning;
                    // Follow a running scan once a second.
                    self.pending_scan = scanning.then(|| std::time::Instant::now() + std::time::Duration::from_secs(1));
                    self.lib.scan = Some(s);
                    if !scanning {
                        self.lib.cache.retain(|k, _| !k.starts_with("local"));
                        if matches!(&self.lib.page, Page::List { sid, .. } if sid == "local") {
                            let p = self.lib.page.clone();
                            self.lib_open(p, false);
                        }
                    }
                    if self.lib.page == Page::Settings {
                        self.lib_form();
                    }
                }
            }
        }
        self.mark(Kind::Library);
    }

    fn lib_field_default(&mut self, id: &str, value: &str) {
        if let Some(f) = self.lib.fields.get_mut(id) {
            if f.value.is_empty() && !f.focused {
                f.value = value.to_owned();
                f.end();
            }
        }
    }

    // ---- the tree ---------------------------------------------------------------

    pub fn lib_tree(&mut self) {
        let mut rows = Vec::new();
        let mut targets = Vec::new();
        let here = self.lib.trail.first().cloned().unwrap_or_else(|| self.lib.page.clone());
        let collapsed = self.lib.collapsed.clone();
        let group = |rows: &mut Vec<TreeRow>, targets: &mut Vec<Target>, key: &str, label: &str, depth: u8| -> bool {
            let open = !collapsed.contains(key);
            rows.push(TreeRow { label: label.into(), depth, group: true, open, selected: false });
            targets.push(Target::Toggle(key.into()));
            open
        };
        let leaf = |rows: &mut Vec<TreeRow>, targets: &mut Vec<Target>, t: Target, label: &str, depth: u8| {
            let selected = match (&t, &here) {
                (Target::Open { sid, node, .. }, Page::List { sid: s2, node: n2, .. }) => sid == s2 && node == n2,
                (Target::Search { sid, .. }, Page::Search { sid: s2, .. }) => sid == s2,
                (Target::Settings(None), Page::Settings) => true,
                _ => false,
            };
            rows.push(TreeRow { label: label.into(), depth, group: false, open: false, selected });
            targets.push(t);
        };
        let src = self.lib.sources.clone();
        let source_rows = |rows: &mut Vec<TreeRow>, targets: &mut Vec<Target>, s: &Source, depth: u8| {
            for (node, title) in &s.roots {
                leaf(rows, targets, Target::Open { sid: s.id.clone(), node: node.clone(), title: format!("{} › {title}", s.name) }, title, depth);
            }
            if s.search {
                leaf(rows, targets, Target::Search { sid: s.id.clone(), title: format!("{} › Search", s.name) }, "Search", depth);
            }
            if s.needs_setup {
                let section = if s.id == "spotify" { "Spotify" } else { "SoundCloud" };
                leaf(rows, targets, Target::Settings(Some(section)), "Connect…", depth);
            }
        };
        if let Some(local) = src.iter().find(|s| s.id == "local") {
            if group(&mut rows, &mut targets, "local", "Local Media", 0) {
                source_rows(&mut rows, &mut targets, local, 1);
            }
        }
        if group(&mut rows, &mut targets, "network", "Network & Cloud", 0) {
            let network: Vec<&Source> = src.iter().filter(|s| s.group == "network").collect();
            if network.is_empty() {
                leaf(&mut rows, &mut targets, Target::Settings(Some("Media servers")), "Add a server…", 1);
            }
            for s in network {
                if group(&mut rows, &mut targets, &format!("src:{}", s.id), &s.name, 1) {
                    source_rows(&mut rows, &mut targets, s, 2);
                }
            }
        }
        if group(&mut rows, &mut targets, "internet", "Internet", 0) {
            for s in src.iter().filter(|s| s.group == "internet") {
                if group(&mut rows, &mut targets, &format!("src:{}", s.id), &s.name, 1) {
                    source_rows(&mut rows, &mut targets, s, 2);
                }
            }
        }
        for id in ["soundcloud", "spotify", "skins"] {
            if let Some(s) = src.iter().find(|s| s.id == id) {
                if group(&mut rows, &mut targets, id, &s.name, 0) {
                    source_rows(&mut rows, &mut targets, s, 1);
                }
            }
        }
        leaf(&mut rows, &mut targets, Target::Settings(None), "Settings", 0);
        self.lib.view.tree = rows;
        self.lib.targets = targets;
        self.mark(Kind::Library);
    }

    // ---- pages ----------------------------------------------------------------

    pub fn lib_open(&mut self, page: Page, from_tree: bool) {
        if from_tree {
            self.lib.trail.clear();
        }
        self.lib.page = page.clone();
        self.lib.view.table.selected.clear();
        self.lib.view.table.top = 0;
        self.lib.anchor = None;
        self.lib.sort = None;
        self.lib.filter.clear();
        self.lib.view.back = !self.lib.trail.is_empty();
        self.lib.view.crumb = if self.lib.trail.is_empty() { String::new() } else { match &page { Page::List { title, .. } => title.clone(), _ => String::new() } };
        self.lib.view.form = None;
        self.lib_tree();
        match &page {
            Page::Settings => {
                self.lib.view.search_shown = false;
                if let Some(b) = &self.backend {
                    b.config();
                    b.scan_status();
                }
                self.lib_form();
            }
            Page::Search { sid, .. } => {
                self.lib.view.search_shown = true;
                self.lib.view.search.placeholder = match sid.as_str() {
                    "radio" => "Type a station or genre, or paste a stream address".into(),
                    "podcasts" => "Type a podcast name, or paste a feed address".into(),
                    "soundcloud" => "Paste a SoundCloud link (track, playlist or artist), or search".into(),
                    "spotify" => "Search Spotify: songs, artists, your playlists".into(),
                    "skins" => "Search the Skin Museum: a band, a game, a colour, a name".into(),
                    _ => "Type and press Enter to search".into(),
                };
                self.lib.view.search.value.clear();
                self.lib.view.search.caret = 0;
                self.lib_focus(Some("search".into()));
                if self.lib.cache.contains_key(&page.key()) {
                    self.lib_show();
                } else {
                    self.lib.items.clear();
                    self.lib.hint = None;
                    self.lib_rows();
                }
            }
            Page::List { sid, node, .. } => {
                self.lib.view.search_shown = true;
                self.lib.view.search.placeholder = "Search".into();
                self.lib.view.search.value.clear();
                self.lib.view.search.caret = 0;
                if self.lib.cache.contains_key(&page.key()) {
                    self.lib_show();
                } else {
                    self.lib.items.clear();
                    self.lib.hint = None;
                    self.lib_rows();
                    self.lib.view.status = "Loading…".into();
                }
                // Always refresh; the cached copy shows meanwhile.
                self.lib.loading = true;
                if let Some(b) = &self.backend {
                    b.browse(page.key(), sid.clone(), node.clone());
                }
            }
        }
        self.mark(Kind::Library);
    }

    pub fn lib_show(&mut self) {
        let Some(l) = self.lib.cache.get(&self.lib.page.key()) else { return };
        self.lib.actions = l.actions.clone();
        self.lib.hint = l.hint.clone();
        let mut items = l.items.clone();
        let f = self.lib.filter.to_lowercase();
        if !f.is_empty() {
            items.retain(|i| format!("{} {} {} {}", i.artist, i.title, i.album, i.info).to_lowercase().contains(&f));
        }
        if let Some((col, asc)) = self.lib.sort {
            let key = |i: &Item| -> String {
                match self.lib_columns(&l.items).get(col).map(|c| c.0.as_str()) {
                    Some("Artist" | "Genre") => i.artist.to_lowercase(),
                    Some("Album" | "Format") => i.album.to_lowercase(),
                    Some("Length") => format!("{:010}", i.duration.unwrap_or(0.0) as u64),
                    Some("Items") => format!("{:010}", i.count.unwrap_or(0)),
                    Some("Posted") => i.date.clone(),
                    Some("Plays") => format!("{:012}", i.plays.unwrap_or(0)),
                    Some("Likes") => format!("{:012}", i.likes.unwrap_or(0)),
                    _ => i.title.to_lowercase(),
                }
            };
            items.sort_by_key(|i| key(i));
            if !asc {
                items.reverse();
            }
        }
        self.lib.items = items;
        self.lib_rows();
    }

    fn lib_columns(&self, items: &[Item]) -> Vec<(String, f32)> {
        let c = |l: &str, w: f32| (l.to_owned(), w);
        if !items.is_empty() && items.iter().all(|i| i.kind == museum::KIND) {
            return vec![c("Skin", 3.0), c("File", 2.0)];
        }
        if !items.is_empty() && items.iter().all(|i| !is_playable(i)) {
            vec![c("Name", 3.0), c("Artist", 2.0), c("Items", 0.7)]
        } else if !items.is_empty() && items.iter().all(|i| i.live) {
            vec![c("Station", 3.0), c("Genre", 2.0), c("Format", 1.1)]
        } else if items.iter().any(|i| !i.date.is_empty() || i.plays.is_some()) {
            // SoundCloud: when it went up and how popular it is, in place of the album.
            vec![c("Artist", 1.2), c("Title", 2.3), c("Posted", 1.3), c("Plays", 0.8), c("Likes", 0.75), c("Length", 0.85)]
        } else {
            vec![c("Artist", 2.0), c("Title", 3.0), c("Album", 2.0), c("Length", 0.9)]
        }
    }

    /// Turns the items into table rows, buttons and status.
    fn lib_rows(&mut self) {
        let all = self.lib.cache.get(&self.lib.page.key()).map(|l| l.items.clone()).unwrap_or_default();
        let columns = self.lib_columns(&all);
        let rows: Vec<Row> = self
            .lib
            .items
            .iter()
            .map(|i| {
                let cells = columns
                    .iter()
                    .map(|(c, _)| match c.as_str() {
                        "Artist" | "Genre" => i.artist.clone(),
                        "Album" | "Format" | "File" => i.album.clone(),
                        "Length" => i.duration.map(clock).unwrap_or_default(),
                        "Items" => i.count.map(|n| n.to_string()).unwrap_or_default(),
                        "Posted" => i.date.clone(),
                        "Plays" => i.plays.map(count).unwrap_or_default(),
                        "Likes" => i.likes.map(count).unwrap_or_default(),
                        _ => i.title.clone(),
                    })
                    .collect();
                Row { cells, folder: i.kind == "folder", dim: is_preview(i) || i.info == gamp_core::soundcloud_web::PROTECTED, tip: i.info.clone() }
            })
            .collect();
        let t = &mut self.lib.view.table;
        t.columns = columns;
        t.rows = rows;
        let max_top = self.lib.view.max_top();
        let t = &mut self.lib.view.table;
        t.top = t.top.min(max_top);
        t.empty = match (&self.lib.page, self.lib.loading) {
            (_, true) if self.lib.items.is_empty() => "Loading…".into(),
            (Page::Search { .. }, _) if self.lib.items.is_empty() => self.lib.view.search.placeholder.clone() + ".",
            _ => self.lib.hint.clone().unwrap_or_default(),
        };
        let skins = matches!(&self.lib.page, Page::List { sid, .. } | Page::Search { sid, .. } if sid == "skins");
        let mut buttons: Vec<(String, String)> = if skins { [("skin:use", "Use skin"), ("skin:surprise", "Surprise me"), ("skin:default", "Original skin")] } else { [("play", "Play"), ("enqueue", "Enqueue"), ("playall", "Play all")] }
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        for a in &self.lib.actions {
            buttons.push((format!("action:{}", a.id), a.label.to_owned()));
        }
        self.lib.view.buttons = buttons;
        let total: f64 = self.lib.items.iter().filter_map(|i| i.duration).sum();
        let hours = if total > 3600.0 { format!(" · {:.1} h", total / 3600.0) } else { String::new() };
        let scanning = match (&self.lib.page, &self.lib.scan) {
            (Page::List { sid, .. }, Some(s)) if sid == "local" && s.scanning => format!(" · scanning… {} files", s.scanned),
            _ => String::new(),
        };
        self.lib.view.status = format!("{} items{hours}{scanning}", self.lib.items.len());
        self.mark(Kind::Library);
    }

    // ---- clicks -------------------------------------------------------------------

    pub fn lib_hot(&self, x: i32, y: i32) -> Option<Hot> {
        // Later entries are drawn on top; search them first.
        self.lib.hots.iter().rev().find(|(_, r)| lw::inside(*r, x, y)).map(|(h, _)| h.clone())
    }

    pub fn lib_press(&mut self, x: i32, y: i32) {
        let Some(h) = self.lib_hot(x, y) else {
            if y < 20 {
                self.start_drag(Kind::Library);
            }
            return;
        };
        match &h {
            Hot::Resize => {
                if let Some(w) = self.win(Kind::Library) {
                    self.resize = Some(crate::app::Resize { start: w.global_cursor(), size: self.lib.view.size(), ..Default::default() });
                }
                return;
            }
            Hot::Tree(i) => {
                match self.lib.targets.get(*i).cloned() {
                    Some(Target::Toggle(k)) => {
                        if !self.lib.collapsed.remove(&k) {
                            self.lib.collapsed.insert(k);
                        }
                        self.lib_tree();
                    }
                    Some(Target::Open { sid, node, title }) => self.lib_open(Page::List { sid, node, title }, true),
                    Some(Target::Search { sid, title }) => self.lib_open(Page::Search { sid, title }, true),
                    Some(Target::Settings(section)) => {
                        self.lib_open(Page::Settings, true);
                        self.lib.view.form_scroll = section.and_then(|p| lw::heading_scroll(&self.skin, &self.lib.view, p)).unwrap_or(0);
                    }
                    None => {}
                }
                return;
            }
            Hot::Search => {
                self.lib_focus(Some("search".into()));
                return;
            }
            Hot::Row(i) => {
                let i = *i;
                self.lib_focus(None);
                if self.double_click(Kind::Library, x, y) {
                    self.lib_activate(i);
                    return;
                }
                let sel = &mut self.lib.view.table.selected;
                if self.modifiers.shift_key() {
                    let a = self.lib.anchor.unwrap_or(i);
                    *sel = (a.min(i)..=a.max(i)).collect();
                } else if self.modifiers.control_key() || self.modifiers.super_key() {
                    if !sel.remove(&i) {
                        sel.insert(i);
                    }
                    self.lib.anchor = Some(i);
                } else {
                    *sel = BTreeSet::from([i]);
                    self.lib.anchor = Some(i);
                }
            }
            Hot::Header(c) => self.lib_click_header(*c),
            Hot::Scroll => {
                self.lib.view.table.top = lw::scroll_top_at(&self.lib.view, y);
            }
            Hot::Form(id) => {
                if self.lib.fields.contains_key(id) {
                    self.lib_focus(Some(id.clone()));
                    return;
                }
                self.lib_focus(None);
            }
            _ => {}
        }
        if h == Hot::TitleBar {
            self.start_drag(Kind::Library);
            return;
        }
        self.lib.view.pressed = Some(h);
        self.mark(Kind::Library);
    }

    pub fn lib_drag(&mut self, _x: i32, y: i32) {
        if self.lib.view.pressed == Some(Hot::Scroll) {
            self.lib.view.table.top = lw::scroll_top_at(&self.lib.view, y);
            self.mark(Kind::Library);
        }
    }

    pub fn lib_release(&mut self) {
        let Some(pressed) = self.lib.view.pressed.take() else { return };
        self.mark(Kind::Library);
        let (x, y) = self.win(Kind::Library).map(|w| w.cursor).unwrap_or((0, 0));
        if self.lib_hot(x, y) != Some(pressed.clone()) {
            return;
        }
        match pressed {
            Hot::Close => self.toggle_window(Kind::Library),
            Hot::Back => {
                if let Some(prev) = self.lib.trail.pop() {
                    self.lib_open(prev, false);
                }
            }
            Hot::Button(id) => self.lib_button(&id),
            Hot::Form(id) => self.lib_form_action(&id),
            _ => {}
        }
    }

    pub fn lib_wheel(&mut self, lines: f64) {
        if self.lib.view.form.is_some() {
            let max = lw::max_form_scroll(&self.skin, &self.lib.view);
            self.lib.view.form_scroll = (self.lib.view.form_scroll - (lines * 30.0) as i32).clamp(0, max);
        } else {
            let (x, _) = self.win(Kind::Library).map(|w| w.cursor).unwrap_or((0, 0));
            if x < 11 + 152 {
                let top = self.lib.view.tree_top as isize - (lines * 2.0) as isize;
                let max = self.lib.view.tree.len().saturating_sub(self.lib.view.tree_rows());
                self.lib.view.tree_top = top.clamp(0, max as isize) as usize;
            } else {
                let top = self.lib.view.table.top as isize - (lines * 3.0).round() as isize;
                self.lib.view.table.top = top.clamp(0, self.lib.view.max_top() as isize) as usize;
            }
        }
        self.mark(Kind::Library);
    }

    /// Double-click or Enter: open a folder, or play from that track.
    fn lib_activate(&mut self, i: usize) {
        let Some(item) = self.lib.items.get(i).cloned() else { return };
        if item.kind == museum::KIND {
            return self.skin_fetch(Some(item));
        }
        if !is_playable(&item) {
            let sid = match &self.lib.page {
                Page::List { sid, .. } | Page::Search { sid, .. } => sid.clone(),
                Page::Settings => return,
            };
            self.lib.trail.push(self.lib.page.clone());
            self.lib_open(Page::List { sid, node: item.id.clone(), title: item.title.clone() }, false);
            return;
        }
        // A station plays alone; a song plays with the ones after it.
        let tracks: Vec<Item> = if item.live { vec![item] } else { self.lib.items[i..].iter().filter(|t| is_playable(t)).cloned().collect() };
        self.lib_send(tracks, false);
    }

    fn lib_selected(&self) -> Vec<Item> {
        self.lib.view.table.selected.iter().filter_map(|i| self.lib.items.get(*i).cloned()).collect()
    }

    /// Puts tracks in the playlist (replacing it, or at the end).
    pub fn lib_send(&mut self, items: Vec<Item>, enqueue: bool) {
        let entries: Vec<Entry> = items
            .into_iter()
            .filter(is_playable)
            .filter(|i| i.info != gamp_core::soundcloud_web::PROTECTED)
            .filter_map(|i| {
                let url = i.url.clone()?;
                // A station shows by its name alone, as SHOUTcast did.
                let artist = if i.live { String::new() } else { i.artist.clone() };
                // A SoundCloud preview stops after 30 seconds: say so where it plays.
                let (title, secs) = if is_preview(&i) { (format!("{} (preview)", i.title), Some(30.0)) } else { (i.title.clone(), i.duration) };
                Some(Entry::remote(url, title, artist, secs, i.live))
            })
            .take(5000)
            .collect();
        if entries.is_empty() {
            self.lib.view.status = "Open it to play what's inside.".into();
            return;
        }
        if enqueue && !self.playlist.entries.is_empty() {
            self.playlist.append(entries);
            self.sync_playlist();
            self.queue_after(self.playlist.current);
        } else if enqueue {
            // Added to an empty list: ready, not playing.
            self.playlist.set(entries);
            self.sync_playlist();
            self.update_title();
        } else {
            self.playlist.set(entries);
            self.sync_playlist();
            self.load_current(true);
        }
    }

    /// Right-click on a track: select it (unless it's in the selection)
    /// and offer Play / Add to playlist.
    pub fn lib_row_menu(&mut self) -> Option<Vec<gamp_ui::menu::Item>> {
        use gamp_ui::menu::Item as M;
        let (x, y) = self.win(Kind::Library)?.cursor;
        let Some(Hot::Row(i)) = self.lib_hot(x, y) else { return None };
        if !self.lib.view.table.selected.contains(&i) {
            self.lib.view.table.selected = BTreeSet::from([i]);
            self.lib.anchor = Some(i);
            self.mark(Kind::Library);
        }
        if self.lib_selected().iter().any(|i| i.kind == museum::KIND) {
            return Some(vec![M::action("lib:skin", "Use skin"), M::action("skin:surprise", "Surprise me"), M::action("skin:default", "Original skin")]);
        }
        if !self.lib_selected().iter().any(is_playable) {
            return None;
        }
        let mut menu = vec![M::action("lib:play", "Play"), M::action("lib:enqueue", "Add to playlist (Shift+Enter)")];
        if let Some(t) = self.lib.items.get(i).filter(|t| !t.artist_link.is_empty()) {
            let who = t.artist_link.rsplit('/').next().unwrap_or_default();
            menu.push(M::action("lib:artist", &format!("Latest from {who}")));
        }
        Some(menu)
    }

    pub fn lib_click_header(&mut self, c: usize) {
        // Dates and counts start with the newest or biggest.
        let all = self.lib.cache.get(&self.lib.page.key()).map(|l| l.items.clone()).unwrap_or_default();
        let up = !matches!(self.lib_columns(&all).get(c).map(|c| c.0.as_str()), Some("Posted" | "Plays" | "Likes" | "Length" | "Items"));
        self.lib.sort = match self.lib.sort {
            Some((col, asc)) if col == c => Some((c, !asc)),
            _ => Some((c, up)),
        };
        self.lib_show();
    }

    /// Downloads a museum skin (or a random one) and puts it on.
    pub fn skin_fetch(&mut self, item: Option<Item>) {
        let Some(b) = &self.backend else { return };
        b.skin(item, self.data_dir.join(MUSEUM_DIR));
        self.say("FETCHING SKIN…");
    }

    /// The Skin Museum's best-loved skins, in the library window.
    pub fn skin_museum(&mut self) {
        if !self.wins.get(&Kind::Library).is_some_and(|w| w.visible) {
            self.toggle_window(Kind::Library);
        }
        self.lib_open(Page::List { sid: "skins".into(), node: "popular".into(), title: "Skin Museum › Most popular".into() }, true);
    }

    /// An uploader's tracks, newest first (what their SoundCloud link lists).
    pub fn lib_artist(&mut self) {
        let Some(link) = self.lib.view.table.selected.first().and_then(|i| self.lib.items.get(*i)).map(|t| t.artist_link.clone()) else { return };
        if link.is_empty() {
            return;
        }
        self.lib_open(Page::Search { sid: "soundcloud".into(), title: "SoundCloud › Search".into() }, true);
        self.lib.view.search.value = link;
        self.lib_submit("search");
        // Listed to pick from, not played.
        self.pending_stream = false;
    }

    pub fn lib_button(&mut self, id: &str) {
        match id {
            "play" | "enqueue" => {
                let sel = self.lib_selected();
                if sel.is_empty() {
                    self.lib.view.status = "Select some tracks first.".into();
                } else {
                    self.lib_send(sel, id == "enqueue");
                }
            }
            "playall" => self.lib_send(self.lib.items.clone(), false),
            "skin:use" => match self.lib_selected().into_iter().next() {
                Some(i) => self.skin_fetch(Some(i)),
                None => self.lib.view.status = "Select a skin first.".into(),
            },
            "skin:surprise" | "skin:default" => self.menu_action(id, None),
            other => {
                if let Some(action) = other.strip_prefix("action:") {
                    let sel = self.lib_selected();
                    let sid = match &self.lib.page {
                        Page::List { sid, .. } | Page::Search { sid, .. } => sid.clone(),
                        Page::Settings => return,
                    };
                    if sel.is_empty() {
                        self.lib.view.status = "Select something first.".into();
                    } else if let Some(b) = &self.backend {
                        b.action(sid, action.to_owned(), sel);
                    }
                }
            }
        }
        self.mark(Kind::Library);
    }

    // ---- typing -------------------------------------------------------------------

    pub fn lib_focus(&mut self, id: Option<String>) {
        self.lib.focus = id.clone();
        self.lib.view.search.focused = id.as_deref() == Some("search");
        for (k, f) in self.lib.fields.iter_mut() {
            f.focused = id.as_deref() == Some(k.as_str());
        }
        if self.lib.page == Page::Settings {
            self.lib_form();
        }
        self.mark(Kind::Library);
    }

    fn focused_field(&mut self) -> Option<&mut Field> {
        match self.lib.focus.as_deref() {
            Some("search") => Some(&mut self.lib.view.search),
            Some(id) => self.lib.fields.get_mut(id),
            None => None,
        }
    }

    /// A key in the library window; true if it was used for typing.
    pub fn lib_key(&mut self, key: &winit::keyboard::Key, text: Option<&str>) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let cmd = self.modifiers.control_key() || self.modifiers.super_key();
        if self.lib.focus.is_none() {
            // Arrows and Enter in the table.
            match key {
                // Shift+Enter adds the selection to the playlist.
                Key::Named(NamedKey::Enter) if self.modifiers.shift_key() => {
                    self.lib_button("enqueue");
                    return true;
                }
                Key::Named(NamedKey::Enter) => {
                    if let Some(i) = self.lib.view.table.selected.iter().next().copied() {
                        self.lib_activate(i);
                    }
                    return true;
                }
                Key::Named(NamedKey::Backspace) if !self.lib.trail.is_empty() => {
                    let prev = self.lib.trail.pop().unwrap();
                    self.lib_open(prev, false);
                    return true;
                }
                Key::Named(d @ (NamedKey::ArrowDown | NamedKey::ArrowUp)) => {
                    let n = self.lib.items.len();
                    if n == 0 {
                        return true;
                    }
                    let at = self.lib.anchor.unwrap_or(0) as isize + if *d == NamedKey::ArrowDown { 1 } else { -1 };
                    let i = at.clamp(0, n as isize - 1) as usize;
                    self.lib.view.table.selected = BTreeSet::from([i]);
                    self.lib.anchor = Some(i);
                    let rows = self.lib.view.visible_rows();
                    let t = &mut self.lib.view.table;
                    if i < t.top {
                        t.top = i;
                    } else if i >= t.top + rows {
                        t.top = i + 1 - rows;
                    }
                    self.mark(Kind::Library);
                    return true;
                }
                Key::Character(c) if cmd && c.as_str() == "a" => {
                    self.lib.view.table.selected = (0..self.lib.items.len()).collect();
                    self.mark(Kind::Library);
                    return true;
                }
                _ => return false,
            }
        }
        let focus = self.lib.focus.clone().unwrap_or_default();
        match key {
            Key::Named(NamedKey::Enter) => {
                self.lib_submit(&focus);
            }
            Key::Named(NamedKey::Escape) => self.lib_focus(None),
            Key::Named(NamedKey::Tab) => {
                // Next field in the form.
                let order = ["srv_name", "srv_url", "srv_user", "srv_pass", "srv_otp"];
                if let Some(i) = order.iter().position(|o| *o == focus) {
                    self.lib_focus(Some(order[(i + 1) % order.len()].into()));
                }
            }
            Key::Named(NamedKey::Backspace) => {
                if let Some(f) = self.focused_field() {
                    f.backspace();
                }
            }
            Key::Named(NamedKey::Delete) => {
                if let Some(f) = self.focused_field() {
                    f.delete();
                }
            }
            Key::Named(NamedKey::ArrowLeft) => {
                if let Some(f) = self.focused_field() {
                    f.move_caret(-1);
                }
            }
            Key::Named(NamedKey::ArrowRight) => {
                if let Some(f) = self.focused_field() {
                    f.move_caret(1);
                }
            }
            Key::Named(NamedKey::Home) => {
                if let Some(f) = self.focused_field() {
                    f.caret = 0;
                }
            }
            Key::Named(NamedKey::End) => {
                if let Some(f) = self.focused_field() {
                    f.end();
                }
            }
            Key::Character(c) if cmd && c.as_str() == "v" => {
                if let Some(t) = crate::clipboard::get() {
                    if let Some(f) = self.focused_field() {
                        f.insert(t.lines().next().unwrap_or(""));
                    }
                }
            }
            Key::Character(c) if cmd && c.as_str() == "a" => {
                if let Some(f) = self.focused_field() {
                    f.end();
                }
            }
            _ => {
                if let Some(t) = text.filter(|t| !cmd && !t.chars().any(char::is_control)) {
                    if let Some(f) = self.focused_field() {
                        f.insert(t);
                    }
                }
            }
        }
        // Typing in a list's search box filters it as you go.
        if focus == "search" && matches!(self.lib.page, Page::List { .. }) {
            self.lib.filter = self.lib.view.search.value.clone();
            self.lib_show();
        }
        if self.lib.page == Page::Settings {
            self.lib_form();
        }
        self.mark(Kind::Library);
        true
    }

    pub(crate) fn lib_submit(&mut self, focus: &str) {
        match focus {
            "search" => {
                if let Page::Search { sid, .. } = self.lib.page.clone() {
                    let q = self.lib.view.search.value.trim().to_owned();
                    if q.is_empty() {
                        return;
                    }
                    self.lib.loading = true;
                    self.lib.view.status = "Searching…".into();
                    self.lib.cache.remove(&self.lib.page.key());
                    self.lib.items.clear();
                    self.lib_rows();
                    if let Some(b) = &self.backend {
                        b.search(self.lib.page.key(), sid.clone(), q.clone());
                    }
                    // A pasted stream address or SoundCloud link plays straight away.
                    let link = q.starts_with("http://") || q.starts_with("https://");
                    self.pending_stream = link && (sid == "radio" || gamp_core::soundcloud_web::is_link(&q));
                }
            }
            id if id.starts_with("srv_") => self.lib_form_action("srv:add"),
            "folder_path" => self.lib_form_action("folder:path"),
            "share" => self.lib_form_action("share:connect"),
            "sc_profile" => self.lib_form_action("sc:save"),
            _ => {}
        }
    }

    // ---- settings -----------------------------------------------------------------

    fn field(&self, id: &str) -> W {
        W::Field(id.to_owned(), self.lib.fields.get(id).cloned().unwrap_or_default())
    }

    fn value(&self, id: &str) -> String {
        self.lib.fields.get(id).map(|f| f.value.trim().to_owned()).unwrap_or_default()
    }

    fn clear(&mut self, ids: &[&str]) {
        for id in ids {
            if let Some(f) = self.lib.fields.get_mut(*id) {
                f.value.clear();
                f.caret = 0;
            }
        }
    }

    pub fn lib_form(&mut self) {
        let c = self.lib.config.clone();
        let platform = c["platform"].as_str().unwrap_or(std::env::consts::OS).to_owned();
        let mut f = vec![W::Heading("Music folders and network drives".into())];
        f.push(W::Text(match platform.as_str() {
            "macos" => "Add any folder. Network drives work once mounted in Finder (Go › Connect to Server, or use Connect below); they appear under /Volumes.",
            "windows" => "Add any folder, a mapped drive, or a \\\\server\\share path.",
            _ => "Add any folder, including network shares mounted under /mnt or /media.",
        }
        .into()));
        let folders: Vec<String> = c["folders"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        if folders.is_empty() {
            f.push(W::Text("No folders yet.".into()));
        }
        for (i, p) in folders.iter().enumerate() {
            f.push(W::Item { id: format!("folder:rm:{i}"), label: p.clone(), button: "Remove".into() });
        }
        let scan = match &self.lib.scan {
            Some(s) if s.scanning => format!("Scanning… {} files, {} tracks", s.scanned, s.found),
            Some(s) => format!("{} tracks", s.found),
            None => String::new(),
        };
        f.push(W::Row(vec![W::button("folder:add", "Add folder…"), W::button("folder:rescan", "Rescan"), W::Text(scan)]));
        f.push(W::Row(vec![self.field("folder_path"), W::button("folder:path", "Add path")]));
        if matches!(platform.as_str(), "macos" | "windows" | "linux") {
            f.push(W::Row(vec![self.field("share"), W::button("share:connect", "Connect to share")]));
        }

        f.push(W::Heading("Media servers (at home or over the internet)".into()));
        f.push(W::Text("Plex, Jellyfin, Navidrome and other Subsonic servers, Synology Audio Station, WebDAV and Nextcloud. DLNA servers on your network appear by themselves.".into()));
        for s in self.lib.sources.iter().filter(|s| s.group == "network" && !s.id.starts_with("dlna-") && !s.id.starts_with("dropbox-") && !s.id.starts_with("cloud-")) {
            f.push(W::Item { id: format!("srv:rm:{}", s.id), label: s.name.clone(), button: "Remove".into() });
        }
        f.push(W::Row(vec![W::button("plex:signin", "Sign in with Plex"), W::Text("Adds every Plex server on your account.".into())]));
        let (kid, klabel, kph) = KINDS[self.lib.kind % KINDS.len()];
        if let Some(fl) = self.lib.fields.get_mut("srv_url") {
            fl.placeholder = kph.into();
        }
        if let Some(fl) = self.lib.fields.get_mut("srv_pass") {
            fl.placeholder = if kid == "plex" { "X-Plex-Token".into() } else { "Password".into() };
        }
        f.push(W::Row(vec![W::button("srv:kind", &format!("Type: {klabel} ›")), self.field("srv_name")]));
        f.push(W::Row(vec![self.field("srv_url")]));
        if kid == "synology" {
            f.push(W::Row(vec![self.field("srv_user"), self.field("srv_pass"), self.field("srv_otp")]));
        } else if kid == "plex" {
            f.push(W::Row(vec![self.field("srv_pass")]));
        } else {
            f.push(W::Row(vec![self.field("srv_user"), self.field("srv_pass")]));
        }
        f.push(W::Row(vec![W::Check { id: "srv:insecure".into(), label: "Accept a self-signed certificate".into(), checked: self.lib.insecure }, W::button("srv:add", "Add server")]));
        f.push(W::Row(vec![W::button("dlna:search", "Search the network again")]));

        f.push(W::Heading("Dropbox, Google Drive and OneDrive".into()));
        // Accounts signed in through Dropbox's web API (older versions).
        for s in self.lib.sources.iter().filter(|s| s.id.starts_with("dropbox-")) {
            f.push(W::Item { id: format!("srv:rm:{}", s.id), label: s.name.clone(), button: "Disconnect".into() });
        }
        let cloud: Vec<String> = c["cloud"].as_array().map(|a| a.iter().filter_map(|v| v["name"].as_str().map(str::to_owned)).collect()).unwrap_or_default();
        f.push(W::Text("Install the Dropbox, Google Drive or OneDrive app and sign in: its folder shows up under Network & Cloud by itself. Songs download as you play them.".into()));
        f.push(W::Row(vec![
            W::button("cloud:get", "Get Dropbox"),
            W::button("cloud:look", "Look again"),
            W::Text(if cloud.is_empty() { "None on this computer yet".into() } else { format!("Found: {}", cloud.join(", ")) }),
        ]));

        f.push(W::Heading("Spotify (Premium)".into()));
        f.push(W::Text("Sign in once in your web browser. Your Liked Songs, playlists and search then show here, and songs play through Gamp's equalizer and visualizers.".into()));
        let sp = c["spotify"]["connected"] == true;
        f.push(W::Row(vec![
            if sp { W::button("sp:disconnect", "Disconnect Spotify") } else { W::button("sp:connect", "Connect Spotify") },
            W::Text(if sp { "Connected" } else { "Not connected" }.into()),
        ]));

        f.push(W::Heading("SoundCloud".into()));
        f.push(W::Text("No account needed: search SoundCloud, or paste any track, playlist or artist link into Search. For your likes and playlists, add your profile's address (they need to be public).".into()));
        f.push(W::Row(vec![self.field("sc_profile"), W::button("sc:save", "Save")]));

        f.push(W::Heading("Internet radio, podcasts and the Internet Archive".into()));
        f.push(W::Text("Nothing to set up: they're under Internet. Paste any stream address into Internet Radio › Search to play it.".into()));
        f.push(W::Heading("Skins".into()));
        f.push(W::Text("Drop any classic Winamp .wsz skin on the player, or right-click › Skins.".into()));
        self.lib.view.form = Some(f);
        self.mark(Kind::Library);
    }

    fn lib_form_action(&mut self, id: &str) {
        let Some(b) = &self.backend else { return };
        let folders: Vec<String> = self.lib.config["folders"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect()).unwrap_or_default();
        match id {
            "folder:add" => {
                if let Some(dir) = rfd::FileDialog::new().set_title("Add a music folder").pick_folder() {
                    let mut f = folders.clone();
                    f.push(dir.to_string_lossy().into_owned());
                    b.post("/api/config", json!({ "folders": f }), true);
                    b.post("/api/library/rescan", json!({}), false);
                    b.scan_status();
                }
            }
            "folder:path" => {
                let p = self.value("folder_path");
                if !p.is_empty() {
                    let mut f = folders.clone();
                    f.push(p);
                    b.post("/api/config", json!({ "folders": f }), true);
                    b.post("/api/library/rescan", json!({}), false);
                    self.clear(&["folder_path"]);
                }
            }
            "folder:rescan" => {
                b.rescan();
                self.lib.view.status = "Scanning…".into();
            }
            "share:connect" => {
                let url = self.value("share");
                b.post("/api/mount", json!({ "url": url }), false);
                b.note(if cfg!(target_os = "macos") { "Finder is connecting. Then Add folder… › /Volumes." } else { "Connecting. Then add the share's folder." });
            }
            "srv:kind" => self.lib.kind = (self.lib.kind + 1) % KINDS.len(),
            "srv:insecure" => self.lib.insecure = !self.lib.insecure,
            "srv:add" => {
                let (kid, _, _) = KINDS[self.lib.kind % KINDS.len()];
                let body = json!({
                    "kind": kid,
                    "name": self.value("srv_name"),
                    "url": self.value("srv_url"),
                    "username": self.value("srv_user"),
                    "password": self.lib.fields.get("srv_pass").map(|f| f.value.clone()).unwrap_or_default(),
                    "otp": self.value("srv_otp"),
                    "insecure": self.lib.insecure,
                });
                b.post("/api/accounts", body, true);
                b.note("Connecting…");
                self.clear(&["srv_pass", "srv_otp"]);
            }
            "plex:signin" => open_in_browser(&b.url("/auth/plex/login")),
            "dlna:search" => {
                b.post("/api/sources/discover", json!({}), false);
                b.note("Looking for DLNA servers…");
                self.pending_refresh = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
            }
            "cloud:get" => open_in_browser("https://www.dropbox.com/install"),
            "cloud:look" => {
                b.refresh_sources();
                b.config();
            }
            "sp:connect" => open_in_browser(&b.url("/auth/spotify/login")),
            "sp:disconnect" => b.post("/api/spotify/logout", json!({}), true),
            "sc:save" => b.post("/api/config", json!({ "soundcloudProfile": self.value("sc_profile") }), true),
            other => {
                if let Some(i) = other.strip_prefix("folder:rm:").and_then(|i| i.parse::<usize>().ok()) {
                    let f: Vec<String> = folders.iter().enumerate().filter(|(n, _)| *n != i).map(|(_, p)| p.clone()).collect();
                    b.post("/api/config", json!({ "folders": f }), true);
                } else if let Some(sid) = other.strip_prefix("srv:rm:") {
                    b.delete_account(sid.to_owned());
                } else if let Some(text) = other.strip_prefix("copy:") {
                    crate::clipboard::set(text);
                    b.note("Copied");
                }
            }
        }
        self.lib_form();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn counts_read_at_a_glance() {
        let c: Vec<String> = [7, 950, 4321, 9_999, 45_600, 120_000, 1_234_567, 3_000_000, 12_345_678].into_iter().map(super::count).collect();
        assert_eq!(c, ["7", "950", "4.3K", "10K", "46K", "120K", "1.2M", "3M", "12M"]);
    }

    use std::time::{Duration, Instant};

    use gamp_audio::OutputKind;
    use gamp_ui::main_window::PlayState;

    use super::*;
    use crate::backend::Backend;

    fn tone(path: &std::path::Path) {
        let rate = 22050u32;
        let frames = rate * 3;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + frames * 2).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(frames * 2).to_le_bytes());
        for i in 0..frames {
            b.extend_from_slice(&(((i as f32 * 0.05).sin() * 8000.0) as i16).to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    fn wait(app: &mut App, secs: u64, mut done: impl FnMut(&App) -> bool) {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            app.tick();
            if done(app) {
                return;
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        panic!("timed out: config {} scan {:?} status {:?} items {} state {:?} msg {:?}", app.lib.config, app.lib.scan.as_ref().map(|s| (s.scanning, s.found)), app.lib.view.status, app.lib.items.len(), app.main.state, app.main.message);
    }

    /// Online: the Skin Museum lists, a skin downloads and goes on, Surprise
    /// me picks another, Original goes back, Keep puts one in the Skins menu.
    #[test]
    #[ignore]
    fn skin_museum_round_trip() {
        let root = std::env::temp_dir().join(format!("gamp-skin-test-{}", std::process::id()));
        // SAFETY: this test runs alone (it's ignored by default).
        unsafe { std::env::set_var("WINAMP_PORT", "8794") };
        let backend = Backend::start(root.join("data"), None).unwrap();
        let mut app = App::new(OutputKind::Null, Some(backend)).unwrap();
        app.data_dir = root.join("data");
        app.lib_start();
        app.lib_open(Page::List { sid: "skins".into(), node: "popular".into(), title: "Skin Museum".into() }, true);
        wait(&mut app, 30, |a| a.lib.items.len() > 100);
        assert_eq!(app.lib.view.table.columns[0].0, "Skin");
        assert_eq!(app.lib.view.buttons[0].1, "Use skin");
        app.lib_activate(0);
        let museum = app.data_dir.join(MUSEUM_DIR);
        wait(&mut app, 60, |a| a.skin_path.as_ref().is_some_and(|p| p.starts_with(&museum)));
        let first = app.skin_path.clone().unwrap();
        app.menu_action("skin:surprise", None);
        wait(&mut app, 60, |a| a.skin_path.as_ref().is_some_and(|p| *p != first));
        app.menu_action("skin:default", None);
        assert!(app.skin_path.is_none());
        app.load_skin(Some(first.clone()));
        assert!(app.main_menu_has("Keep this skin"));
        app.menu_action("skin:keep", None);
        assert!(app.skin_path.as_ref().unwrap().starts_with(app.data_dir.join("skins")));
        assert!(!app.main_menu_has("Keep this skin"));
        // Searching works too.
        app.lib_open(Page::Search { sid: "skins".into(), title: "Skin Museum › Search".into() }, true);
        app.lib.view.search.value = "metal".into();
        app.lib_submit("search");
        wait(&mut app, 30, |a| !a.lib.items.is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_folder_plays_through_the_library_and_the_server() {
        let root = std::env::temp_dir().join(format!("gamp-lib-test-{}", std::process::id()));
        let music = root.join("Music");
        std::fs::create_dir_all(&music).unwrap();
        tone(&music.join("Library Tone.wav"));
        // SAFETY: only this test in the binary sets the port.
        unsafe { std::env::set_var("WINAMP_PORT", "8793") };
        let backend = Backend::start(root.join("data"), None).unwrap();
        let mut app = App::new(OutputKind::Null, Some(backend)).unwrap();
        app.lib_start();
        let b = app.backend.as_ref().unwrap();
        b.post("/api/config", json!({ "folders": [music.to_string_lossy()] }), true);
        b.post("/api/library/rescan", json!({}), false);
        // The scan finishes and the local list shows the track.
        wait(&mut app, 10, |a| a.lib.config["folders"].as_array().is_some_and(|f| !f.is_empty()));
        app.backend.as_ref().unwrap().rescan();
        wait(&mut app, 15, |a| a.lib.scan.as_ref().is_some_and(|s| !s.scanning && s.found == 1));
        app.lib_open(Page::List { sid: "local".into(), node: "audio".into(), title: "Local".into() }, true);
        wait(&mut app, 10, |a| !a.lib.items.is_empty());
        assert_eq!(app.lib.items[0].title, "Library Tone");
        assert_eq!(app.lib.view.table.rows.len(), 1);
        // The tree lists the built-in internet sources and Settings.
        assert!(app.lib.view.tree.iter().any(|r| r.label == "Internet Radio"));
        assert!(app.lib.view.tree.iter().any(|r| r.label == "Settings"));
        // Play it: through the local server, the HTTP reader and the engine.
        app.lib_send(app.lib.items.clone(), false);
        wait(&mut app, 10, |a| a.main.state == PlayState::Playing && a.main.elapsed > 0.3);
        assert_eq!(app.main.khz, Some(22));
        assert!(app.main.duration.is_some_and(|d| (d - 3.0).abs() < 0.1));
        // Seeking works over HTTP too.
        app.player.seek(2.0);
        wait(&mut app, 5, |a| a.main.elapsed >= 2.0);
        // The settings form builds from the server's config.
        app.lib_open(Page::Settings, true);
        let form = app.lib.view.form.clone().unwrap();
        assert!(form.iter().any(|w| matches!(w, W::Item { label, .. } if label.ends_with("Music"))));
        let _ = std::fs::remove_dir_all(&root);
    }
}

fn is_preview(i: &Item) -> bool {
    i.info.starts_with("SoundCloud only allows")
}

/// 950 → "950"; 4321 → "4.3K"; 45600 → "46K"; 1234567 → "1.2M".
fn count(n: u64) -> String {
    let short = |v: f64, unit: &str| if v < 10.0 { format!("{v:.1}{unit}").replace(".0", "") } else { format!("{v:.0}{unit}") };
    match n {
        0..=999 => n.to_string(),
        1_000..=999_999 => short(n as f64 / 1e3, "K"),
        _ => short(n as f64 / 1e6, "M"),
    }
}
