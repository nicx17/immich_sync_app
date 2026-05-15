//! Asset upload flow including duplicate detection, checksum computation, and retry logic.

use std::path::Path;
use std::time::Duration;

use futures_util::TryStreamExt;

use super::errors::{RequestContext, classify_http_issue, classify_network_issue};
use super::upload_helpers::{
    apply_asset_timezone_fixup, file_timestamps, local_timezone_name, mime_for_path,
    unix_to_utc_iso8601,
};
use super::{ApiIssue, ImmichApiClient, TransferProgressCallback};

impl ImmichApiClient {
    pub async fn upload_asset(
        &self,
        file_path: &str,
        checksum: &str,
        progress: Option<TransferProgressCallback>,
    ) -> Option<String> {
        let base_url = match self.get_active_url().await {
            Some(u) => u,
            None => {
                log::error!("No active connection. Skipping upload: {}", file_path);
                self.set_issue(ApiIssue {
                    summary: "No active server connection".to_string(),
                    guidance: "Test the server connection in Settings and confirm at least one Immich URL is reachable."
                        .to_string(),
                })
                .await;
                return None;
            }
        };

        let path = Path::new(file_path);
        if !path.exists() {
            log::warn!("File not found, skipping: {}", file_path);
            self.set_issue(ApiIssue {
                summary: "A queued file is no longer available".to_string(),
                guidance: "Check that the watched folder still exists and that the file was not moved or deleted before upload."
                    .to_string(),
            })
            .await;
            return None;
        }

        let meta = match std::fs::metadata(path) {
            Ok(m) => m,
            Err(e) => {
                log::error!("Could not read metadata for {}: {}", file_path, e);
                self.set_issue(ApiIssue {
                    summary: "Mimick could not read a queued file".to_string(),
                    guidance: "Verify folder permissions and make sure the file is still accessible to the app."
                        .to_string(),
                })
                .await;
                return None;
            }
        };

        let (created_ts, modified_ts) = file_timestamps(&meta);
        let created_at = unix_to_utc_iso8601(created_ts);
        let modified_at = unix_to_utc_iso8601(modified_ts);
        let desired_time_zone = local_timezone_name();
        let filename = path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "upload".to_string());
        let device_asset_id = format!("mimick-rust-{}", checksum);
        let device_id = "mimick-rust-client".to_string();
        let mime = mime_for_path(path);

        log::info!("Uploading: {} ({} bytes)", file_path, meta.len());
        log::debug!(
            "  device_asset_id={}, created={}",
            device_asset_id,
            created_at
        );
        let file_len = meta.len();

        // Stream the file body so large videos do not get buffered into memory.
        let file = match tokio::fs::File::open(path).await {
            Ok(f) => f,
            Err(e) => {
                log::error!("Failed to open {}: {}", file_path, e);
                self.set_issue(ApiIssue {
                    summary: "Mimick could not open a queued file".to_string(),
                    guidance: "The file may be locked, deleted, or outside the app's allowed folder access."
                        .to_string(),
                })
                .await;
                return None;
            }
        };

        let progress_for_stream = progress.clone();
        let mut uploaded_bytes = 0_u64;
        let stream = tokio_util::codec::FramedRead::new(file, tokio_util::codec::BytesCodec::new())
            .inspect_ok(move |chunk| {
                uploaded_bytes = uploaded_bytes.saturating_add(chunk.len() as u64);
                if let Some(callback) = &progress_for_stream {
                    callback(uploaded_bytes, Some(file_len));
                }
            });
        let file_body = reqwest::Body::wrap_stream(stream);

        let file_part = reqwest::multipart::Part::stream_with_length(file_body, file_len)
            .file_name(filename.clone())
            .mime_str(mime)
            .ok()?;

        let form = reqwest::multipart::Form::new()
            .part("assetData", file_part)
            .text("deviceAssetId", device_asset_id)
            .text("deviceId", device_id)
            .text("fileCreatedAt", created_at)
            .text("fileModifiedAt", modified_at)
            .text("isFavorite", "false");

        let url = format!("{}/api/assets", base_url);
        let api_key = self.settings.read().api_key.clone();

        match self
            .client
            .post(&url)
            .header("x-api-key", &api_key)
            .header("Accept", "application/json")
            .multipart(form)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status().as_u16();
                match status {
                    200 | 201 => {
                        if let Ok(json) = resp.json::<serde_json::Value>().await {
                            let asset_id = json["id"].as_str().map(String::from);
                            if let Some(callback) = &progress {
                                callback(file_len, Some(file_len));
                            }
                            if let Some(asset_id) = asset_id.as_deref() {
                                self.schedule_asset_timezone_fixup(
                                    base_url.clone(),
                                    asset_id.to_string(),
                                    desired_time_zone.clone(),
                                );
                            }
                            self.clear_issue().await;
                            log::info!("Upload OK: {} => {:?}", filename, asset_id);
                            asset_id
                        } else {
                            log::warn!(
                                "Upload returned {} but body unreadable: {}",
                                status,
                                filename
                            );
                            None
                        }
                    }
                    409 => {
                        log::info!("Duplicate (already in Immich): {}", filename);
                        self.clear_issue().await;
                        if let Some(callback) = &progress {
                            callback(file_len, Some(file_len));
                        }
                        // Some versions return the ID even on 409
                        if let Ok(json) = resp.json::<serde_json::Value>().await
                            && let Some(id) = json["id"].as_str()
                        {
                            return Some(id.to_string());
                        }
                        Some("DUPLICATE".to_string())
                    }
                    413 => {
                        log::error!("Upload failed (file too large): {}", filename);
                        self.set_issue(ApiIssue {
                            summary: "Immich rejected a file as too large".to_string(),
                            guidance: "Reduce the file size, raise the server's upload limits, or use a folder rule to skip oversized files."
                                .to_string(),
                        })
                        .await;
                        None
                    }
                    401 | 403 => {
                        self.set_issue(ApiIssue {
                            summary: "Immich rejected the API key".to_string(),
                            guidance: "Update the API key in Settings and ensure it has the Asset upload + update and Album read/create/addAsset permissions."
                                .to_string(),
                        })
                        .await;
                        None
                    }
                    502..=504 => {
                        log::warn!("Server error {}: retrying later for {}", status, filename);
                        let mut active = self.active_url.lock().await;
                        *active = None;
                        self.set_issue(ApiIssue {
                            summary: "Immich is temporarily unavailable".to_string(),
                            guidance: "Wait a moment and retry. If it keeps happening, check the server logs and reverse proxy."
                                .to_string(),
                        })
                        .await;
                        None
                    }
                    _ => {
                        let body = resp.text().await.unwrap_or_default();
                        log::error!("Upload failed [{}] for {}: {}", status, filename, body);
                        self.set_issue(classify_http_issue(
                            RequestContext::Upload,
                            status,
                            Some(&filename),
                        ))
                        .await;
                        None
                    }
                }
            }
            Err(e) => {
                log::error!("Network error uploading {}: {}", filename, e);
                // Force connection re-check on next upload
                let mut active = self.active_url.lock().await;
                *active = None;
                self.set_issue(classify_network_issue(RequestContext::Upload, &e))
                    .await;
                None
            }
        }
    }

    // --------------- Album Management ---------------

    fn schedule_asset_timezone_fixup(
        &self,
        base_url: String,
        asset_id: String,
        time_zone: Option<String>,
    ) {
        let client = self.client.clone();
        let api_key = self.settings.read().api_key.clone();

        tokio::spawn(async move {
            let Some(time_zone) = time_zone else {
                log::warn!(
                    "Could not determine local timezone for uploaded asset {}; leaving Immich timezone unchanged",
                    asset_id
                );
                return;
            };

            // Immich can rewrite the timeline placement after the initial upload once
            // the metadata extraction job finishes. Re-apply the intended timezone
            // after a few short delays so the final stored value matches the source file.
            for delay_secs in [2_u64, 8, 20] {
                tokio::time::sleep(Duration::from_secs(delay_secs)).await;
                apply_asset_timezone_fixup(&client, &api_key, &base_url, &asset_id, &time_zone)
                    .await;
            }
        });
    }
}
