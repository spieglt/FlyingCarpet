use crate::utils::BluetoothMessage;

use tokio::sync::mpsc;

pub(crate) struct BluetoothPeripheral {
    tx: mpsc::Sender<BluetoothMessage>,
}
