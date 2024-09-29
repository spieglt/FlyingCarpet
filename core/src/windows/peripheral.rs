use tokio::sync::mpsc;

use crate::bluetooth::{PASSWORD_CHARACTERISTIC_UUID, SERVICE_UUID, SSID_CHARACTERISTIC_UUID};
use windows::{
    core::{Interface, Result, GUID, HSTRING},
    Devices::Bluetooth::{
        BluetoothError,
        GenericAttributeProfile::{
            GattCharacteristicProperties, GattLocalCharacteristic,
            GattLocalCharacteristicParameters, GattProtectionLevel, GattReadRequestedEventArgs,
            GattServiceProvider, GattServiceProviderAdvertisementStatusChangedEventArgs,
            GattServiceProviderAdvertisingParameters, GattWriteRequestedEventArgs,
        },
    },
    Foundation::TypedEventHandler,
    Storage::Streams::{DataReader, DataWriter, UnicodeEncoding},
};

use super::BluetoothMessage;

pub(crate) struct BluetoothPeripheral {
    tx: mpsc::Sender<BluetoothMessage>,
    service_provider: GattServiceProvider,
    _wifi_information: Option<String>,
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
            _wifi_information: None,
        })
    }

    pub fn add_characteristic(&mut self) -> Result<bool> {
        // create characteristics
        let gatt_operand_parameters = GattLocalCharacteristicParameters::new()?;
        gatt_operand_parameters.SetCharacteristicProperties(GattCharacteristicProperties::Read)?;
        gatt_operand_parameters.SetCharacteristicProperties(GattCharacteristicProperties::Write)?;
        gatt_operand_parameters
            .SetReadProtectionLevel(GattProtectionLevel::EncryptionAndAuthenticationRequired)?;
        gatt_operand_parameters.SetUserDescription(&HSTRING::from("Flying Carpet"))?;

        // make ssid characteristic
        // for characteristic in (SSID_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID) {
        let result = self
            .service_provider
            .Service()?
            .CreateCharacteristicAsync(SSID_CHARACTERISTIC_UUID.into(), &gatt_operand_parameters)?
            .get()?;
        if result.Error()? != BluetoothError::Success {
            println!(
                "Failed to create GattLocalCharacteristic: {:?}",
                result.Error()?
            );
            return Ok(false);
        }
        let ssid_characteristic = result.Characteristic()?;

        // ssid read handler
        let ssid_read_callback =
            TypedEventHandler::<GattLocalCharacteristic, GattReadRequestedEventArgs>::new(
                move |_gatt_local_characteristic, gatt_read_requested_event_args| {
                    let args = gatt_read_requested_event_args
                        .as_ref()
                        .expect("No args in read callback");
                    // let deferral = args.GetDeferral()?;
                    let request = args.GetRequestAsync()?.get()?;
                    let writer = DataWriter::new()?;
                    writer.WriteBytes(b"oh yeah")?;
                    request.RespondWithValue(&writer.DetachBuffer()?)?;
                    // deferral.Complete()?;
                    Ok(())
                },
            );
        ssid_characteristic.ReadRequested(&ssid_read_callback)?;

        // ssid write handler
        let ssid_write_callback =
            TypedEventHandler::<GattLocalCharacteristic, GattWriteRequestedEventArgs>::new(
                move |_gatt_local_characteristic, gatt_write_requested_event_args| {
                    let args = gatt_write_requested_event_args
                        .as_ref()
                        .expect("No args in read callback");
                    let request = args.GetRequestAsync()?.get()?;
                    // get value
                    unsafe {
                        let data_reader = DataReader::from_raw(request.Value()?.as_raw());
                        data_reader.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
                        let ssid = data_reader.ReadString(request.Value()?.Length()?)?;
                        // got_ssid(ssid.to_string());
                    }
                    // deferral.Complete()?;
                    Ok(())
                },
            );
        ssid_characteristic.WriteRequested(&ssid_write_callback)?;

        Ok(true)
    }

    pub fn start_advertising(&mut self) -> Result<()> {
        // make service connectable and discoverable
        let adv_parameters = GattServiceProviderAdvertisingParameters::new()?;
        adv_parameters.SetIsConnectable(true)?;
        adv_parameters.SetIsDiscoverable(true)?;

        // start advertising
        let advertisement_status_changed_callback = TypedEventHandler::<
            GattServiceProvider,
            GattServiceProviderAdvertisementStatusChangedEventArgs,
        >::new(|sender, _args| {
            println!(
                "Advertisement status: {:?}",
                sender
                    .as_ref()
                    .expect("No sender in advertisement status changed callback")
                    .AdvertisementStatus()
            );
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
