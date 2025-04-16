use super::{fc_error, ibuffer_to_string, NO_SSID};
use crate::bluetooth::{
    OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID, SERVICE_UUID, SSID_CHARACTERISTIC_UUID,
};
use crate::utils::BluetoothMessage;
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use windows::{
    core::{Result, GUID, HSTRING},
    Devices::Bluetooth::{
        BluetoothError,
        GenericAttributeProfile::{
            GattCharacteristicProperties, GattLocalCharacteristic,
            GattLocalCharacteristicParameters, GattProtectionLevel, GattReadRequestedEventArgs,
            GattServiceProvider, GattServiceProviderAdvertisementStatus,
            GattServiceProviderAdvertisementStatusChangedEventArgs,
            GattServiceProviderAdvertisingParameters, GattWriteRequestedEventArgs,
        },
    },
    Foundation::TypedEventHandler,
    Storage::Streams::DataWriter,
};

type CharacteristicReadHandler =
    TypedEventHandler<GattLocalCharacteristic, GattReadRequestedEventArgs>;
type CharacteristicWriteHandler =
    TypedEventHandler<GattLocalCharacteristic, GattWriteRequestedEventArgs>;

pub(crate) struct BluetoothPeripheral {
    tx: mpsc::Sender<BluetoothMessage>,
    service_provider: GattServiceProvider,
    // ssid and password fields are set by main thread if we're hosting, so peer can read these.
    // if we're joining and peer is writing wifi info to us, we'll write those details back to
    // the main thread with tx.
    pub ssid: Arc<Mutex<Option<String>>>,
    pub password: Arc<Mutex<Option<String>>>,
    pub connection_ready: Arc<Mutex<bool>>,
}

impl BluetoothPeripheral {
    pub fn new(tx: mpsc::Sender<BluetoothMessage>) -> Result<Self> {
        // create service provider
        let result = GattServiceProvider::CreateAsync(GUID::from(SERVICE_UUID))?.get()?;
        if result.Error()? != BluetoothError::Success {
            println!(
                "Failed to create GattServiceProvider: {:?}",
                result.Error()?
            );
            result.Error()?;
        }
        let service_provider = result.ServiceProvider()?;
        Ok(BluetoothPeripheral {
            tx,
            service_provider,
            ssid: Arc::new(Mutex::new(None)),
            password: Arc::new(Mutex::new(None)),
            connection_ready: Arc::new(Mutex::new(false)),
        })
    }

    pub fn add_characteristics(&mut self) -> std::result::Result<(), super::FCError> {
        // create characteristics
        let gatt_operand_parameters = GattLocalCharacteristicParameters::new()?;
        gatt_operand_parameters.SetCharacteristicProperties(
            GattCharacteristicProperties::Read | GattCharacteristicProperties::Write,
        )?;
        gatt_operand_parameters
            .SetReadProtectionLevel(GattProtectionLevel::EncryptionAndAuthenticationRequired)?;
        gatt_operand_parameters
            .SetWriteProtectionLevel(GattProtectionLevel::EncryptionAndAuthenticationRequired)?;
        gatt_operand_parameters.SetUserDescription(&HSTRING::from("Flying Carpet"))?; // TODO: set this for each characteristic?

        // let local_service = self.service_provider.Service()?;

        // make OS characteristic
        let result = self
            .service_provider
            .Service()?
            .CreateCharacteristicAsync(OS_CHARACTERISTIC_UUID.into(), &gatt_operand_parameters)?
            .get()?;
        let e = result.Error()?;
        if e != BluetoothError::Success {
            fc_error(&format!("Error creating characteristic: {:?}", e))?;
        }
        let os_characteristic = result.Characteristic()?;

        // make SSID characteristic
        let result = self
            .service_provider
            .Service()?
            .CreateCharacteristicAsync(SSID_CHARACTERISTIC_UUID.into(), &gatt_operand_parameters)?
            .get()?;
        let e = result.Error()?;
        if e != BluetoothError::Success {
            fc_error(&format!("Error creating characteristic: {:?}", e))?;
        }
        let ssid_characteristic = result.Characteristic()?;

        // make password characteristic
        let result = self
            .service_provider
            .Service()?
            .CreateCharacteristicAsync(
                PASSWORD_CHARACTERISTIC_UUID.into(),
                &gatt_operand_parameters,
            )?
            .get()?;
        let e = result.Error()?;
        if e != BluetoothError::Success {
            fc_error(&format!("Error creating characteristic: {:?}", e))?;
        }
        let password_characteristic = result.Characteristic()?;

        // OS read handler: write "windows" to peer
        let os_read_callback = CharacteristicReadHandler::new(
            move |_gatt_local_characteristic, gatt_read_requested_event_args| {
                println!("received os read request");
                let args = gatt_read_requested_event_args
                    .as_ref()
                    .expect("No args in read callback");
                let deferral = args.GetDeferral()?;
                let request = args.GetRequestAsync()?.get()?;
                let writer = DataWriter::new()?;
                writer.WriteBytes(b"windows")?;
                request.RespondWithValue(&writer.DetachBuffer()?)?;
                deferral.Complete()?;
                println!("wrote OS to central");
                Ok(())
            },
        );
        os_characteristic.ReadRequested(&os_read_callback)?;

        // OS write handler: send peer's OS back to main thread so that it can decide if we're starting or joining hotspot
        let os_write_tx = self.tx.clone();
        let os_write_callback = CharacteristicWriteHandler::new(
            move |_gatt_local_characteristic, gatt_write_requested_event_args| {
                println!("received os write request");
                let args = gatt_write_requested_event_args
                    .as_ref()
                    .expect("No args in write callback");
                let deferral = args.GetDeferral()?;
                let request = args.GetRequestAsync()?.get()?;
                let ibuffer = request.Value()?;
                let peer_os = ibuffer_to_string(ibuffer)?;
                if let Err(e) = os_write_tx.blocking_send(BluetoothMessage::PeerOS(peer_os)) {
                    println!("Could not send on Bluetooth tx: {}", e);
                };
                request.Respond()?;
                deferral.Complete()?;
                Ok(())
            },
        );
        os_characteristic.WriteRequested(&os_write_callback)?;

        // ssid read handler
        let callback_ssid = self.ssid.clone();
        let callback_tx = self.tx.clone();
        let connection_ready = self.connection_ready.clone();
        let ssid_read_callback = CharacteristicReadHandler::new(
            move |_gatt_local_characteristic, gatt_read_requested_event_args| {
                println!("received ssid read request");
                let args = gatt_read_requested_event_args
                    .as_ref()
                    .expect("No args in read callback");
                let deferral = args.GetDeferral()?;
                let request = args.GetRequestAsync()?.get()?;
                let writer = DataWriter::new()?;
                let callback_ssid = callback_ssid.blocking_lock();
                let ssid = match callback_ssid.as_ref() {
                    Some(_ssid) => _ssid.to_string(),
                    None => NO_SSID.to_string(),
                };
                writer.WriteBytes(ssid.as_bytes())?;
                request.RespondWithValue(&writer.DetachBuffer()?)?;
                println!("peer read our ssid: {}", ssid);
                if ssid != NO_SSID {
                    // Set connection ready flag to indicate iOS device has started the connection process
                    {
                        let mut is_ready = connection_ready.blocking_lock();
                        *is_ready = true;
                    }
                    
                    if let Err(e) = callback_tx.blocking_send(BluetoothMessage::PeerReadSsid) {
                        println!("Could not send on Bluetooth tx: {}", e);
                    };
                }
                deferral.Complete()?;
                Ok(())
            },
        );
        ssid_characteristic.ReadRequested(&ssid_read_callback)?;

        // ssid write handler
        let callback_tx = self.tx.clone();
        let ssid_write_callback = CharacteristicWriteHandler::new(
            move |_gatt_local_characteristic, gatt_write_requested_event_args| {
                println!("received ssid write request");
                let args = gatt_write_requested_event_args
                    .as_ref()
                    .expect("No args in write callback");
                let deferral = args.GetDeferral()?;
                let request = args.GetRequestAsync()?.get()?;
                // get value
                let ibuffer = request.Value()?;
                let ssid = ibuffer_to_string(ibuffer)?;
                callback_tx
                    .blocking_send(BluetoothMessage::SSID(ssid.to_string()))
                    .expect("Could not send to main thread from SSID write handler");
                request.Respond()?;
                deferral.Complete()?;
                Ok(())
            },
        );
        ssid_characteristic.WriteRequested(&ssid_write_callback)?;

        // password read handler
        let callback_password = self.password.clone();
        let callback_tx = self.tx.clone();
        let connection_ready = self.connection_ready.clone();
        let password_read_callback = CharacteristicReadHandler::new(
            move |_gatt_local_characteristic, gatt_read_requested_event_args| {
                println!("received password read request");
                let args = gatt_read_requested_event_args
                    .as_ref()
                    .expect("No args in read callback");
                let deferral = args.GetDeferral()?;
                let request = args.GetRequestAsync()?.get()?;
                let writer = DataWriter::new()?;
                
                // Check if connection is ready (SSID was previously read)
                let is_ready = {
                    let ready = connection_ready.blocking_lock();
                    *ready
                };
                
                // If the iOS device hasn't read the SSID yet, we need to delay
                // This ensures the connection sequence happens in the correct order
                if !is_ready {
                    // Add a small delay to allow iOS device to process the SSID first
                    std::thread::sleep(std::time::Duration::from_millis(500));
                }
                
                let callback_password = callback_password.blocking_lock();
                let callback_password = match callback_password.as_ref() {
                    Some(p) => p,
                    None => &"".to_string(),
                };
                writer.WriteBytes(callback_password.as_bytes())?;
                request.RespondWithValue(&writer.DetachBuffer()?)?;
                println!("peer read our password: {}", callback_password);
                
                // Send a more detailed message about the connection stage
                if let Err(e) = callback_tx.blocking_send(BluetoothMessage::PeerReadPassword) {
                    println!("Could not send on Bluetooth tx: {}", e);
                };
                deferral.Complete()?;
                Ok(())
            },
        );
        password_characteristic.ReadRequested(&password_read_callback)?;

        // password write handler
        let callback_tx = self.tx.clone();
        let password_write_callback = CharacteristicWriteHandler::new(
            move |_gatt_local_characteristic, gatt_write_requested_event_args| {
                println!("received password write request");
                let args = gatt_write_requested_event_args
                    .as_ref()
                    .expect("No args in write callback");
                let deferral = args.GetDeferral()?;
                let request = args.GetRequestAsync()?.get()?;
                // get value
                let ibuffer = request.Value()?;
                let password = ibuffer_to_string(ibuffer)?;
                callback_tx
                    .blocking_send(BluetoothMessage::Password(password.to_string()))
                    .expect("Could not send to main thread from password write handler");
                request.Respond()?;
                deferral.Complete()?;
                Ok(())
            },
        );
        password_characteristic.WriteRequested(&password_write_callback)?;

        Ok(())
    }

    pub fn start_advertising(&mut self) -> Result<()> {
        // get tx so we can tell main thread we've paired
        let thread_tx = self.tx.clone();

        // make service connectable and discoverable
        let adv_parameters = GattServiceProviderAdvertisingParameters::new()?;
        adv_parameters.SetIsConnectable(true)?;
        adv_parameters.SetIsDiscoverable(true)?;

        // start advertising
        let advertisement_status_changed_callback = TypedEventHandler::<
            GattServiceProvider,
            GattServiceProviderAdvertisementStatusChangedEventArgs,
        >::new(move |sender, _args| {
            let advertisement_status = sender
                .as_ref()
                .expect("No sender in advertisement status changed callback")
                .AdvertisementStatus()?;
            println!("Advertisement status: {:?}", advertisement_status);
            match advertisement_status {
                GattServiceProviderAdvertisementStatus::Created => {
                    println!("Advertisement created")
                }
                GattServiceProviderAdvertisementStatus::Started
                | GattServiceProviderAdvertisementStatus::StartedWithoutAllAdvertisementData => {
                    // TODO: have to worry about StartedWithoutAllAdvertisementData case?
                    thread_tx
                        .blocking_send(BluetoothMessage::StartedAdvertising)
                        .expect("Could not send on Bluetooth tx");
                }
                GattServiceProviderAdvertisementStatus::Aborted => {
                    println!("Advertisement aborted")
                }
                GattServiceProviderAdvertisementStatus::Stopped => {
                    println!("Advertisement stopped")
                }
                _ => println!(
                    "Invalid GattServiceProviderAdvertisementStatus: {}",
                    advertisement_status.0
                ),
            }
            Ok(())
        });
        // TODO: save event registration token here, only used to deregister event later?
        self.service_provider
            .AdvertisementStatusChanged(&advertisement_status_changed_callback)?;
        self.service_provider
            .StartAdvertisingWithParameters(&adv_parameters)?;
        Ok(())
    }
}
