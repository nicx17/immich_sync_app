//! Search and explore: smart search, OCR, metadata filters, people, places, server stats.
//!
//! Wraps the Immich `/search` and `/people` endpoints used by the library
//! explore tab and search bar. Supports CLIP-based smart search, OCR text
//! matching, and structured metadata filters with pagination.

use std::time::Duration;

use super::errors::{RequestContext, classify_http_issue, classify_network_issue};
use super::{
    ExifSearchResponse, ExploreSection, ImmichApiClient, LibraryAsset, MetadataSearchFilters,
    PeopleResponse, Person, PlaceItem, SearchResponse, ServerAbout, ServerStatistics, ServerStats,
    SortOrder,
};

impl ImmichApiClient {
    /// Fetch the list of recognized people faces from the server.
    pub async fn fetch_people(&self, include_hidden: bool) -> Result<Vec<Person>, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!(
            "{}/api/people?withHidden={}",
            base_url,
            if include_hidden { "true" } else { "false" }
        );
        match self
            .client
            .get(&url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let body = resp
                    .json::<PeopleResponse>()
                    .await
                    .map_err(|err| err.to_string())?;
                self.clear_issue().await;
                Ok(body.people)
            }
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(err) => Err(err.to_string()),
        }
    }

    /// Sectioned tile data (places + things) for the Explore landing.
    pub async fn fetch_explore(&self) -> Result<Vec<ExploreSection>, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}/api/search/explore", base_url);
        match self
            .client
            .get(&url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let sections = resp
                    .json::<Vec<ExploreSection>>()
                    .await
                    .map_err(|err| err.to_string())?;
                self.clear_issue().await;
                Ok(sections)
            }
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(err) => Err(err.to_string()),
        }
    }

    /// Fetch all unique cities that have at least one asset with EXIF city data.
    /// Pages through `/api/search/metadata` collecting one representative asset
    /// per city. Caps at 500 pages to bound runtime on very large libraries.
    pub async fn fetch_all_places(&self) -> Result<Vec<PlaceItem>, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}/api/search/metadata", base_url);

        let mut seen: std::collections::HashMap<String, String> = std::collections::HashMap::new();
        let mut page: u32 = 1;
        const PAGE_SIZE: u32 = 250;
        const MAX_PAGES: u32 = 500;

        loop {
            let body = serde_json::json!({
                "withExif": true,
                "page": page,
                "size": PAGE_SIZE,
            });
            let resp = match self
                .client
                .post(&url)
                .header("x-api-key", &settings.api_key)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json")
                .json(&body)
                .timeout(Duration::from_secs(30))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => r,
                Ok(r) => return Err(format!("HTTP {}", r.status())),
                Err(e) => return Err(e.to_string()),
            };
            let parsed: ExifSearchResponse = match resp.json().await {
                Ok(p) => p,
                Err(e) => return Err(e.to_string()),
            };
            let has_more = parsed.assets.next_page.is_some();
            for asset in parsed.assets.items {
                if let Some(city) = asset.exif_info.as_ref().and_then(|e| e.city.clone()) {
                    seen.entry(city).or_insert(asset.id);
                }
            }
            if !has_more || page >= MAX_PAGES {
                break;
            }
            page += 1;
        }

        let mut places: Vec<PlaceItem> = seen
            .into_iter()
            .map(|(city, asset_id)| PlaceItem { city, asset_id })
            .collect();
        places.sort_by(|a, b| a.city.cmp(&b.city));
        Ok(places)
    }

    /// Per-person face thumbnail. Distinct from `fetch_thumbnail` (asset).
    pub async fn fetch_person_thumbnail(&self, person_id: &str) -> Result<Vec<u8>, String> {
        let _permit = self
            .thumbnail_semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|err| err.to_string())?;
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}/api/people/{}/thumbnail", base_url, person_id);
        match self
            .client
            .get(&url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/octet-stream")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let bytes = resp.bytes().await.map_err(|err| err.to_string())?;
                Ok(bytes.to_vec())
            }
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(err) => Err(err.to_string()),
        }
    }

    /// Perform a CLIP embedding-based smart search for matching assets.
    pub async fn search_smart(
        &self,
        query: &str,
        page: u32,
        size: u32,
    ) -> Result<(Vec<LibraryAsset>, bool), String> {
        let body = serde_json::json!({
            "query": query,
            "page": page,
            "size": size.max(1),
        });
        self.fetch_search_assets(
            "/api/search/smart",
            body,
            RequestContext::SmartSearch,
            Some(query),
        )
        .await
    }

    /// Perform an OCR-based search to match recognized text inside library images.
    pub async fn search_ocr(
        &self,
        query: &str,
        page: u32,
        size: u32,
        order: Option<SortOrder>,
    ) -> Result<(Vec<LibraryAsset>, bool), String> {
        let mut body = serde_json::json!({
            "ocr": query,
            "page": page,
            "size": size.max(1),
        });
        if let Some(order) = order
            && let Some(obj) = body.as_object_mut()
        {
            obj.insert(
                "order".into(),
                serde_json::to_value(order).unwrap_or(serde_json::Value::Null),
            );
        }
        self.fetch_search_assets(
            "/api/search/metadata",
            body,
            RequestContext::MetadataSearch,
            Some(query),
        )
        .await
    }

    /// Search library assets matching specific text within their original filenames.
    pub async fn search_metadata(
        &self,
        query: &str,
        page: u32,
        size: u32,
        order: Option<SortOrder>,
    ) -> Result<(Vec<LibraryAsset>, bool), String> {
        let filters = MetadataSearchFilters {
            original_file_name: Some(query.to_string()),
            order,
            ..Default::default()
        };
        self.search_metadata_with_filters(&filters, page, size)
            .await
    }

    /// Perform a advanced search matching specific metadata filters.
    pub async fn search_metadata_with_filters(
        &self,
        filters: &MetadataSearchFilters,
        page: u32,
        size: u32,
    ) -> Result<(Vec<LibraryAsset>, bool), String> {
        let mut body = serde_json::to_value(filters).map_err(|err| err.to_string())?;
        if let Some(obj) = body.as_object_mut() {
            obj.insert("page".into(), serde_json::json!(page));
            obj.insert("size".into(), serde_json::json!(size.max(1)));
        }
        let label = filters.original_file_name.as_deref().unwrap_or("");
        self.fetch_search_assets(
            "/api/search/metadata",
            body,
            RequestContext::MetadataSearch,
            Some(label),
        )
        .await
    }

    /// Retrieve total images, videos, and overall asset count statistics from the server.
    pub async fn fetch_server_stats(&self) -> Result<ServerStats, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}/api/assets/statistics", base_url);

        match self
            .client
            .get(&url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let stats = resp
                    .json::<ServerStats>()
                    .await
                    .map_err(|err| err.to_string())?;
                self.clear_issue().await;
                Ok(stats)
            }
            Ok(resp) => {
                self.set_issue(classify_http_issue(
                    RequestContext::ServerStats,
                    resp.status().as_u16(),
                    None,
                ))
                .await;
                Err(format!("HTTP {}", resp.status()))
            }
            Err(err) => {
                *self.active_url.lock().await = None;
                self.set_issue(classify_network_issue(RequestContext::ServerStats, &err))
                    .await;
                Err(err.to_string())
            }
        }
    }

    /// Fetch server-wide statistics including per-user usage breakdown.
    ///
    /// Admin-only endpoint; non-admin sessions will receive an HTTP 403 and an
    /// `Err` is returned. Caller should fall back to per-user asset counts.
    pub async fn fetch_server_statistics(&self) -> Result<ServerStatistics, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}/api/server/statistics", base_url);

        match self
            .client
            .get(&url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => resp
                .json::<ServerStatistics>()
                .await
                .map_err(|err| err.to_string()),
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(err) => Err(err.to_string()),
        }
    }

    /// Retrieve detailed Immich server system information (e.g. version).
    pub async fn fetch_server_about(&self) -> Result<ServerAbout, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}/api/server/about", base_url);

        match self
            .client
            .get(&url)
            .header("x-api-key", &settings.api_key)
            .header("Accept", "application/json")
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let about = resp
                    .json::<ServerAbout>()
                    .await
                    .map_err(|err| err.to_string())?;
                self.clear_issue().await;
                Ok(about)
            }
            Ok(resp) => {
                self.set_issue(classify_http_issue(
                    RequestContext::ServerAbout,
                    resp.status().as_u16(),
                    None,
                ))
                .await;
                Err(format!("HTTP {}", resp.status()))
            }
            Err(err) => {
                *self.active_url.lock().await = None;
                self.set_issue(classify_network_issue(RequestContext::ServerAbout, &err))
                    .await;
                Err(err.to_string())
            }
        }
    }

    /// Shared internal helper executing paginated POST search queries against asset endpoints.
    pub(super) async fn fetch_search_assets(
        &self,
        endpoint: &str,
        mut body: serde_json::Value,
        context: RequestContext,
        subject: Option<&str>,
    ) -> Result<(Vec<LibraryAsset>, bool), String> {
        if let Some(obj) = body.as_object_mut() {
            obj.entry("withExif")
                .or_insert(serde_json::Value::Bool(true));
        }
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!("{}{}", base_url, endpoint);

        match self
            .client
            .post(&url)
            .header("x-api-key", &settings.api_key)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .json(&body)
            .timeout(Duration::from_secs(10))
            .send()
            .await
        {
            Ok(resp) if resp.status().is_success() => {
                let response = resp
                    .json::<SearchResponse>()
                    .await
                    .map_err(|err| err.to_string())?;
                self.clear_issue().await;
                let has_more = response.assets.next_page.is_some();
                Ok((response.assets.items, has_more))
            }
            Ok(resp) => {
                self.set_issue(classify_http_issue(
                    context,
                    resp.status().as_u16(),
                    subject,
                ))
                .await;
                Err(format!("HTTP {}", resp.status()))
            }
            Err(err) => {
                *self.active_url.lock().await = None;
                self.set_issue(classify_network_issue(context, &err)).await;
                Err(err.to_string())
            }
        }
    }
}
