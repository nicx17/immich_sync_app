//! HTTP and network error classification for user-facing diagnostics.

use super::ApiIssue;

#[derive(Debug, Clone, Copy)]
pub(super) enum RequestContext {
    Upload,
    Albums,
    AlbumCreate,
    AlbumAssign,
    ThumbnailFetch,
    AssetList,
    SmartSearch,
    MetadataSearch,
    AssetDownload,
    ServerStats,
    ServerAbout,
}

pub(super) fn classify_http_issue(
    context: RequestContext,
    status: u16,
    subject: Option<&str>,
) -> ApiIssue {
    match status {
        401 | 403 => ApiIssue {
            summary: "Immich rejected the API key".to_string(),
            guidance: "Update the API key in Settings and confirm it still has the required Asset and Album permissions (see README for the full list)."
                .to_string(),
        },
        404 if matches!(context, RequestContext::AlbumAssign | RequestContext::AlbumCreate) => {
            ApiIssue {
                summary: "An album reference is no longer valid".to_string(),
                guidance: "Refresh the album list or choose a different album before retrying."
                    .to_string(),
            }
        }
        413 => ApiIssue {
            summary: "Immich rejected a file as too large".to_string(),
            guidance: "Reduce the file size, raise the server upload limit, or skip oversized files with folder rules."
                .to_string(),
        },
        429 => ApiIssue {
            summary: "Immich rate-limited the request".to_string(),
            guidance: "Wait a moment and retry. If this happens often, lower upload concurrency or check reverse proxy limits."
                .to_string(),
        },
        502..=504 => ApiIssue {
            summary: "Immich is temporarily unavailable".to_string(),
            guidance: "Wait a moment and retry. If it keeps happening, inspect the server and reverse proxy logs."
                .to_string(),
        },
        _ => ApiIssue {
            summary: match context {
                RequestContext::Upload => {
                    format!("Immich could not accept {}", subject.unwrap_or("the upload"))
                }
                RequestContext::Albums => "Immich could not load the album list".to_string(),
                RequestContext::AlbumCreate => format!(
                    "Immich could not create album '{}'",
                    subject.unwrap_or("Unnamed")
                ),
                RequestContext::AlbumAssign => {
                    "Immich could not add the asset to the selected album".to_string()
                }
                RequestContext::ThumbnailFetch => {
                    "Immich could not load a library thumbnail".to_string()
                }
                RequestContext::AssetList => {
                    "Immich could not load library assets".to_string()
                }
                RequestContext::SmartSearch => {
                    "Immich could not run the smart library search".to_string()
                }
                RequestContext::MetadataSearch => {
                    "Immich could not run the metadata library search".to_string()
                }
                RequestContext::AssetDownload => {
                    "Immich could not download the selected asset".to_string()
                }
                RequestContext::ServerStats => {
                    "Immich could not load library statistics".to_string()
                }
                RequestContext::ServerAbout => {
                    "Immich could not load server version information".to_string()
                }
            },
            guidance: format!(
                "The server responded with HTTP {}. Check the server logs and retry after confirming the current configuration.",
                status
            ),
        },
    }
}

pub(super) fn classify_network_issue(context: RequestContext, error: &reqwest::Error) -> ApiIssue {
    if error.is_timeout() {
        ApiIssue {
            summary: "The Immich request timed out".to_string(),
            guidance: "Check network quality and server responsiveness, then retry.".to_string(),
        }
    } else if error.is_connect() {
        ApiIssue {
            summary: "Could not reach the Immich server".to_string(),
            guidance: "Check the configured URLs, your network connection, and whether the server is online."
                .to_string(),
        }
    } else {
        ApiIssue {
            summary: match context {
                RequestContext::Upload => "The upload request failed before completion".to_string(),
                RequestContext::Albums => "The album request failed before completion".to_string(),
                RequestContext::AlbumCreate => {
                    "The album creation request failed before completion".to_string()
                }
                RequestContext::AlbumAssign => {
                    "The album assignment request failed before completion".to_string()
                }
                RequestContext::ThumbnailFetch => {
                    "The thumbnail request failed before completion".to_string()
                }
                RequestContext::AssetList => {
                    "The library asset request failed before completion".to_string()
                }
                RequestContext::SmartSearch => {
                    "The smart search request failed before completion".to_string()
                }
                RequestContext::MetadataSearch => {
                    "The metadata search request failed before completion".to_string()
                }
                RequestContext::AssetDownload => {
                    "The asset download request failed before completion".to_string()
                }
                RequestContext::ServerStats => {
                    "The library statistics request failed before completion".to_string()
                }
                RequestContext::ServerAbout => {
                    "The server version request failed before completion".to_string()
                }
            },
            guidance: "Retry the request after checking network connectivity and server health."
                .to_string(),
        }
    }
}
