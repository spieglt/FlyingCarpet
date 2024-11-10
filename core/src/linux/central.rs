use bluer::{Adapter, DiscoveryFilter, DiscoveryTransport, Result, Uuid};
use std::collections::HashSet;
use tokio::sync::mpsc;

use super::SERVICE_UUID;
use crate::utils::BluetoothMessage;

// pub(crate) struct BluetoothCentral {
//     tx: mpsc::Sender<BluetoothMessage>,
// }

// impl BluetoothCentral {
//     pub fn new(tx: mpsc::Sender<BluetoothMessage>) -> Result<Self> {
//         Ok(BluetoothCentral {
//             tx,
//         })
//     }

//     pub async fn scan(&mut self) -> bluer::Result<()> {
//         let mut uuids = HashSet::new();
//         uuids.insert(Uuid::parse_str(SERVICE_UUID).expect("Could not parse service UUID"));

//         let filter = DiscoveryFilter {
//             transport: DiscoveryTransport::Le,
//             uuids,
//             ..Default::default()
//         };
//         adapter.set_discovery_filter(filter).await?;
//         println!("Using discovery filter:\n{:#?}\n\n", adapter.discovery_filter().await);
//         Ok(())
//     }
// }

pub async fn scan(adapter: Adapter) -> bluer::Result<()> {
    let mut uuids = HashSet::new();
    uuids.insert(Uuid::parse_str(SERVICE_UUID).expect("Could not parse service UUID"));

    let filter = DiscoveryFilter {
        transport: DiscoveryTransport::Le,
        uuids,
        ..Default::default()
    };
    adapter.set_discovery_filter(filter).await?;
    println!(
        "Using discovery filter:\n{:#?}\n\n",
        adapter.discovery_filter().await
    );
    Ok(())
}
