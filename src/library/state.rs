//! Library runtime state used by the built-in asset browser.

use crate::api_client::{
    LibraryAlbum, LibraryAsset, MetadataSearchFilters, ServerAbout, ServerStats,
};

const PAGE_SIZE: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySource {
    AllAssets,
    Timeline,
    /// Random sample via `POST /api/search/random`. Used as the "Explore"
    /// sidebar destination — refresh re-rolls; pagination is meaningless.
    Explore,
    Album {
        id: String,
        name: String,
    },
    SmartSearch {
        query: String,
    },
    MetadataSearch {
        query: String,
    },
    OcrSearch {
        query: String,
    },

    AdvancedSearch {
        filters: Box<MetadataSearchFilters>,
    },
    /// Local watched-folder enumeration only (no remote calls).
    LocalAll,
    /// Filename substring filter applied over the local enumeration.
    LocalSearch {
        query: String,
    },
    /// Remote assets overlayed with sync state from local SyncIndex.
    Unified,
    /// Local filename filter applied over a unified view.
    UnifiedSearch {
        query: String,
    },
    /// Local enumeration scoped to the folder linked to a specific album.
    /// Empty if the album has no linked folder.
    AlbumLocal {
        id: String,
        name: String,
    },
    /// Remote album assets overlayed with sync state from the linked folder.
    AlbumUnified {
        id: String,
        name: String,
    },
}

impl LibrarySource {
    pub fn is_search(&self) -> bool {
        matches!(
            self,
            LibrarySource::SmartSearch { .. }
                | LibrarySource::MetadataSearch { .. }
                | LibrarySource::OcrSearch { .. }
                | LibrarySource::AdvancedSearch { .. }
                | LibrarySource::LocalSearch { .. }
                | LibrarySource::UnifiedSearch { .. }
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibrarySortMode {
    NewestFirst,
    Filename,
    FileType,
    SyncState,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LibraryLoadState {
    Idle,
    Loading,
    Loaded,
    Empty,
    Error(String),
}

#[derive(Clone, Debug, Default)]
pub struct LibraryStatus {
    pub stats: Option<ServerStats>,
    pub about: Option<ServerAbout>,
}

#[derive(Clone, Debug)]
pub struct LibraryState {
    pub source: LibrarySource,
    pub previous_non_search_source: LibrarySource,
    pub sort_mode: LibrarySortMode,
    pub load_state: LibraryLoadState,
    pub selected_asset_id: Option<String>,
    pub albums: Vec<LibraryAlbum>,
    pub assets: Vec<LibraryAsset>,
    pub next_page: u32,
    pub has_more: bool,
    pub page_in_flight: bool,
    pub generation: u64,
    pub status: LibraryStatus,
}

impl Default for LibraryState {
    fn default() -> Self {
        Self {
            source: LibrarySource::AllAssets,
            previous_non_search_source: LibrarySource::AllAssets,
            sort_mode: LibrarySortMode::NewestFirst,
            load_state: LibraryLoadState::Idle,
            selected_asset_id: None,
            albums: Vec::new(),
            assets: Vec::new(),
            next_page: 1,
            has_more: true,
            page_in_flight: false,
            generation: 0,
            status: LibraryStatus::default(),
        }
    }
}

impl LibraryState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn load_albums(&mut self, albums: Vec<LibraryAlbum>) {
        self.albums = albums;
    }

    pub fn load_initial_source(&mut self) -> (u64, LibrarySource, u32) {
        self.switch_source(LibrarySource::AllAssets)
    }

    pub fn switch_source(&mut self, source: LibrarySource) -> (u64, LibrarySource, u32) {
        if !source.is_search() {
            self.previous_non_search_source = source.clone();
        }
        self.source = source.clone();
        self.selected_asset_id = None;
        self.assets.clear();
        self.next_page = 1;
        self.has_more = true;
        self.page_in_flight = true;
        self.generation = self.generation.saturating_add(1);
        self.load_state = LibraryLoadState::Loading;
        (self.generation, source, 1)
    }

    pub fn load_next_page_if_needed(&mut self) -> Option<(u64, LibrarySource, u32)> {
        if self.page_in_flight || !self.has_more {
            return None;
        }

        self.page_in_flight = true;
        Some((self.generation, self.source.clone(), self.next_page))
    }

    pub fn replace_assets(&mut self, generation: u64, items: Vec<LibraryAsset>) -> bool {
        if generation != self.generation {
            return false;
        }

        self.assets = dedup_assets(items);
        self.page_in_flight = false;
        self.next_page = 2;
        self.has_more = self.assets.len() >= PAGE_SIZE;
        self.load_state = if self.assets.is_empty() {
            LibraryLoadState::Empty
        } else {
            LibraryLoadState::Loaded
        };
        self.apply_sort(self.sort_mode.clone());
        true
    }

    pub fn append_assets(&mut self, generation: u64, items: Vec<LibraryAsset>) -> bool {
        if generation != self.generation {
            return false;
        }

        let page_len = items.len();
        self.page_in_flight = false;
        self.assets.extend(items);
        self.assets = dedup_assets(std::mem::take(&mut self.assets));
        if page_len > 0 {
            self.next_page = self.next_page.saturating_add(1);
        }
        self.has_more = page_len >= PAGE_SIZE;
        self.load_state = if self.assets.is_empty() {
            LibraryLoadState::Empty
        } else {
            LibraryLoadState::Loaded
        };
        self.apply_sort(self.sort_mode.clone());
        true
    }

    pub fn apply_sort(&mut self, mode: LibrarySortMode) {
        self.sort_mode = mode;
        match self.sort_mode {
            LibrarySortMode::NewestFirst => self.assets.sort_by(|a, b| {
                b.created_at
                    .cmp(&a.created_at)
                    .then_with(|| a.id.cmp(&b.id))
            }),
            LibrarySortMode::Filename => self.assets.sort_by(|a, b| {
                a.filename
                    .to_ascii_lowercase()
                    .cmp(&b.filename.to_ascii_lowercase())
                    .then_with(|| a.id.cmp(&b.id))
            }),
            LibrarySortMode::FileType => self.assets.sort_by(|a, b| {
                a.mime_type
                    .cmp(&b.mime_type)
                    .then_with(|| a.filename.cmp(&b.filename))
                    .then_with(|| a.id.cmp(&b.id))
            }),
            LibrarySortMode::SyncState => self.assets.sort_by(|a, b| a.id.cmp(&b.id)),
        }
    }

    pub fn clear_search_restore_previous_source(&mut self) -> Option<(u64, LibrarySource, u32)> {
        if self.source.is_search() {
            Some(self.switch_source(self.previous_non_search_source.clone()))
        } else {
            None
        }
    }

    pub fn mark_error(&mut self, generation: u64, message: impl Into<String>) {
        if generation == self.generation {
            self.page_in_flight = false;
            self.load_state = LibraryLoadState::Error(message.into());
        }
    }

    pub fn set_status(&mut self, stats: Option<ServerStats>, about: Option<ServerAbout>) {
        self.status.stats = stats;
        self.status.about = about;
    }
}

fn dedup_assets(items: Vec<LibraryAsset>) -> Vec<LibraryAsset> {
    let mut seen = std::collections::HashSet::new();
    let mut deduped = Vec::with_capacity(items.len());
    for item in items {
        if seen.insert(item.id.clone()) {
            deduped.push(item);
        }
    }
    deduped
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(id: &str, filename: &str) -> LibraryAsset {
        LibraryAsset {
            id: id.into(),
            filename: filename.into(),
            mime_type: "image/jpeg".into(),
            created_at: format!("2024-01-0{}T00:00:00.000Z", id),
            asset_type: "IMAGE".into(),
            thumbhash: None,
            width: Some(10.0),
            height: Some(10.0),
            checksum: None,
        }
    }

    #[test]
    fn test_switch_source_resets_pagination_and_assets() {
        let mut state = LibraryState::new();
        state.assets.push(asset("1", "a.jpg"));
        state.next_page = 9;
        state.has_more = false;

        let (_, source, page) = state.switch_source(LibrarySource::Album {
            id: "album-1".into(),
            name: "Trips".into(),
        });

        assert!(matches!(source, LibrarySource::Album { .. }));
        assert_eq!(page, 1);
        assert!(state.assets.is_empty());
        assert_eq!(state.next_page, 1);
        assert!(state.has_more);
        assert!(state.page_in_flight);
    }

    #[test]
    fn test_stale_generation_results_are_ignored() {
        let mut state = LibraryState::new();
        let (generation, _, _) = state.load_initial_source();
        let newer_generation = state.switch_source(LibrarySource::MetadataSearch {
            query: "cats".into(),
        });

        assert!(!state.replace_assets(generation, vec![asset("1", "a.jpg")]));
        assert!(state.replace_assets(newer_generation.0, vec![asset("2", "b.jpg")]));
        assert_eq!(state.assets.len(), 1);
        assert_eq!(state.assets[0].id, "2");
    }

    #[test]
    fn test_duplicate_page_requests_are_suppressed() {
        let mut state = LibraryState::new();
        let (_, _, _) = state.load_initial_source();
        assert!(state.load_next_page_if_needed().is_none());
        state.page_in_flight = false;
        assert!(state.load_next_page_if_needed().is_some());
        assert!(state.load_next_page_if_needed().is_none());
    }

    #[test]
    fn test_clear_search_restores_previous_non_search_source() {
        let mut state = LibraryState::new();
        state.switch_source(LibrarySource::Album {
            id: "album-1".into(),
            name: "Trips".into(),
        });
        state.switch_source(LibrarySource::SmartSearch {
            query: "sunset".into(),
        });

        let (_, source, page) = state.clear_search_restore_previous_source().unwrap();
        assert!(matches!(source, LibrarySource::Album { .. }));
        assert_eq!(page, 1);
    }
}
