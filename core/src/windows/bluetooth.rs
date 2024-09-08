mod central;
mod peripheral;

use std::{error::Error, sync::mpsc::{self, Sender}};

use peripheral::BluetoothPeripheral;
use windows::{
    core::GUID,
    Devices::Bluetooth::BluetoothLEDevice,
    Storage::Streams::{DataReader, UnicodeEncoding},
};

const SERVICE_UUID: &str = "A70BF3CA-F708-4314-8A0E-5E37C259BE5C";
const OS_CHARACTERISTIC_UUID: &str = "BEE14848-CC55-4FDE-8E9D-2E0F9EC45946";
const SSID_CHARACTERISTIC_UUID: &str = "0D820768-A329-4ED4-8F53-BDF364EDAC75";
const PASSWORD_CHARACTERISTIC_UUID: &str = "E1FA8F66-CF88-4572-9527-D5125A2E0762";
const NO_SSID: &str = "NONE";

struct Bluetooth {
    ssid_wrote: Sender<Result<String, Box<dyn Error>>>,
    password_wrote: Sender<Result<String, Box<dyn Error>>>,
    central: BluetoothCentral,
    peripheral: BluetoothPeripheral,
}

impl Bluetooth {
    fn new(ssid_wrote: Sender<Result<String, Box<dyn Error>>>, password_wrote: Sender<Result<String, Box<dyn Error>>>) -> Result<Self, Box<dyn Error>> {
        if !peripheral::check_support()? {
            Err("Device does not support acting as a Bluetooth LE peripheral")?;
        }
        let peripheral = BluetoothPeripheral::new()?;

        if !central::check_support()? {
            Err("Device does not support acting as a Bluetooth LE central.")?;
        }
        let central = BluetoothCentral::new()?;
        
        Ok(Bluetooth{ssid_wrote, password_wrote, peripheral, central})
    }

    fn initialize_peripheral() {

    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let central = false;
    if central {
        if !central::check_support()? {
            Err("Central role not supported")?;
        }

        // watch for advertisements
        let (tx, rx) = mpsc::channel::<u64>();
        let _watcher = central::watch_for_advertisements(tx)?;

        // receive address of device advertising our service
        let address = rx.recv()?;
        println!("found bluetooth {:12x}", address);

        // pair
        let device = BluetoothLEDevice::FromBluetoothAddressAsync(address)?.get()?;
        // let status = device.RequestAccessAsync()?.await?;
        // println!("status: {:?}", status);
        // let params = device.RequestPreferredConnectionParameters(preferredconnectionparameters)
        // https://stackoverflow.com/a/38704180/9242143
        let device_info = device.DeviceInformation()?;
        central::pair_device(&device_info)?;
        // return Ok(());

        // stop watching for advertisements
        // _watcher.Stop()?;

        // read service
        let services = device.GetGattServicesAsync()?.await?.Services()?;
        for service in services {
            println!("UUID: {:?}", service.Uuid()?);
            if service.Uuid()? == GUID::from(SERVICE_UUID) {
                println!("found service");
                // let x = device.RequestAccessAsync()?.await?;
                // println!("{:?}", x);
                // println!("requested access");
                let characteristics = service
                    .GetCharacteristicsForUuidAsync(GUID::from(SSID_CHARACTERISTIC_UUID))?
                    .await?
                    .Characteristics()?;
                println!("got chars");
                for characteristic in characteristics {
                    let i_buffer = characteristic.ReadValueAsync().ok().unwrap().await?.Value();
                    if i_buffer.is_err() {
                        println!("nothing in buffer");
                        continue;
                    }
                    let i_buffer = i_buffer.unwrap();
                    println!("IBuffer contents: {:?}", i_buffer);
                    let size = i_buffer.Capacity()?;
                    let data_reader = DataReader::FromBuffer(&i_buffer)?;
                    data_reader.SetUnicodeEncoding(UnicodeEncoding::Utf8)?;
                    let data_string = data_reader.ReadString(size)?.to_string();
                    println!("message: {}", data_string);
                }
            }
        }
    } else {
        if !peripheral::check_support()? {
            Err("Peripheral role not supported")?;
        }
        println!("Peripheral role is supported");
        let mut bluetooth_peripheral = match peripheral::BluetoothPeripheral::new()? {
            Some(p) => p,
            None => Err("Could not create service provider")?,
        };
        bluetooth_peripheral.add_characteristic()?;
        bluetooth_peripheral.start_advertising()?;

        let mut user_input = String::new();
        std::io::stdin().read_line(&mut user_input)?;
    }
    Ok(())
}

async fn _connect_to_device(_address: u64) -> Result<(), Box<dyn Error>> {
    Ok(())
}
