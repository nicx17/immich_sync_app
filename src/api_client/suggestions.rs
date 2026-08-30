//! Search suggestion autocomplete from the Immich server.
//!
//! Wraps `GET /search/suggestions` which returns known values for
//! country, state, city, camera make, model, and lens model fields.

use std::time::Duration;

use super::ImmichApiClient;

impl ImmichApiClient {
    /// Fetch autocomplete suggestions for a given metadata dimension.
    ///
    /// `suggestion_type` must be one of: `"country"`, `"state"`, `"city"`,
    /// `"camera-make"`, `"camera-model"`, `"camera-lens-model"`.
    pub async fn fetch_search_suggestions(
        &self,
        suggestion_type: &str,
    ) -> Result<Vec<String>, String> {
        let base_url = self
            .get_active_url()
            .await
            .ok_or_else(|| "No active connection".to_string())?;
        let settings = self.settings_snapshot();
        let url = format!(
            "{}/api/search/suggestions?type={}",
            base_url, suggestion_type
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
                let items: Vec<String> = resp.json().await.map_err(|e| e.to_string())?;
                self.clear_issue().await;
                Ok(items)
            }
            Ok(resp) => Err(format!("HTTP {}", resp.status())),
            Err(err) => Err(err.to_string()),
        }
    }
}
