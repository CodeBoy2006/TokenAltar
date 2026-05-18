use tokio::sync::mpsc;
use tracing::{error, info};

use crate::{db::Database, models::LedgerEvent, state::MetricsState};

pub type LedgerSender = mpsc::Sender<LedgerEvent>;

pub fn spawn_ledger_worker(db: Database, metrics: MetricsState) -> LedgerSender {
    let (tx, mut rx) = mpsc::channel::<LedgerEvent>(4096);
    tokio::spawn(async move {
        info!("ledger worker started");
        while let Some(event) = rx.recv().await {
            metrics.add_tokens(event.usage.total());
            if let Err(err) = db.apply_ledger_event(&event).await {
                error!(?err, request_id = %event.request_id, "failed to apply ledger event");
            }
        }
    });
    tx
}
