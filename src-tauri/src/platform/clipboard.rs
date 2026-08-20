// macOS / Windows / Linux: Clipboard monitoring via arboard [DR-007]
// Security: Raw clipboard content is NEVER stored, NEVER logged, NEVER sent to Gemini automatically [DR-007, ADR-008]

//! # Clipboard Monitor
//!
//! Background clipboard change observer using hash-only in-memory comparison. [DR-007]
//!
//! ## Security rules:
//! 1. Raw clipboard text is hashed and discarded immediately. It is NEVER stored in memory
//!    longer than the instant required to compute the hash.
//! 2. Clipboard content is NEVER logged at any log level.
//! 3. Clipboard content is NEVER sent to Gemini automatically (only if explicitly requested by user).

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
#[allow(unused_imports)]
use tracing::{debug, error, info, warn};

pub struct ClipboardMonitor {
    #[allow(dead_code)]
    last_content_hash: Arc<Mutex<u64>>,
    cancel: CancellationToken,
}

impl ClipboardMonitor {
    /// Start polling clipboard for content changes every `interval_ms` milliseconds.
    pub fn start<F>(interval_ms: u64, on_change: F) -> Self
    where
        F: Fn() + Send + Sync + 'static,
    {
        let last_content_hash = Arc::new(Mutex::new(0u64));
        let last_hash_clone = Arc::clone(&last_content_hash);
        let cancel = CancellationToken::new();
        let cancel_clone = cancel.clone();

        tokio::spawn(async move {
            info!("Clipboard monitor started (polling every {} ms)", interval_ms);
            let mut interval = tokio::time::interval(Duration::from_millis(interval_ms));

            loop {
                tokio::select! {
                    _ = cancel_clone.cancelled() => {
                        info!("Clipboard monitor cancelled");
                        break;
                    }
                    _ = interval.tick() => {
                        // Check clipboard in blocking task since arboard is synchronous
                        let hash_res = tokio::task::spawn_blocking(|| {
                            match arboard::Clipboard::new() {
                                Ok(mut cb) => {
                                    if let Ok(text) = cb.get_text() {
                                        let mut hasher = DefaultHasher::new();
                                        text.hash(&mut hasher);
                                        // Raw text is dropped immediately here
                                        Some(hasher.finish())
                                    } else {
                                        None
                                    }
                                }
                                Err(_) => None,
                            }
                        }).await;

                        if let Ok(Some(current_hash)) = hash_res {
                            if current_hash != 0 {
                                let mut prev = last_hash_clone.lock().unwrap();
                                if *prev != 0 && *prev != current_hash {
                                    debug!("Clipboard content change detected (hash mismatch)");
                                    on_change();
                                }
                                *prev = current_hash;
                            }
                        }
                    }
                }
            }
        });

        Self {
            last_content_hash,
            cancel,
        }
    }

    /// Stop the clipboard background polling loop.
    pub fn stop(self) {
        self.cancel.cancel();
    }
}
