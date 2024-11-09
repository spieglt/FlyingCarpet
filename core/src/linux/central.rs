use crate::utils::BluetoothMessage;

use tokio::sync::mpsc;

pub(crate) struct BluetoothCentral {
    tx: mpsc::Sender<BluetoothMessage>,
}
