use crate::{
    bluetooth::{
        OS, OS_CHARACTERISTIC_UUID, PASSWORD_CHARACTERISTIC_UUID, SERVICE_UUID,
        SSID_CHARACTERISTIC_UUID,
    },
    utils::BluetoothMessage,
};

use bluer::{
    adv::{Advertisement, AdvertisementHandle},
    gatt::local::{
        Application, ApplicationHandle, Characteristic, CharacteristicRead, CharacteristicWrite,
        CharacteristicWriteMethod, ReqError, Service,
    },
    Uuid,
};
use futures::FutureExt;
use tokio::sync::mpsc;

fn get_os_characteristic(tx: mpsc::Sender<BluetoothMessage>) -> Characteristic {
    // when the OS characteristic is read, return the constant
    // when it's written to, return that to calling thread, so we need tx
    let read_tx = tx.clone();
    let write_tx = tx.clone();
    Characteristic {
        uuid: Uuid::parse_str(OS_CHARACTERISTIC_UUID).unwrap(),
        read: Some(CharacteristicRead {
            read: true,
            secure_read: true,
            // so this is a pub type CharacteristicReadFun = Box<dyn Fn(CharacteristicReadRequest) -> Pin<Box<dyn Future<Output = ReqResult<Vec<u8>>> + Send>> + Send + Sync>;
            // a box containing function, that takes a characteristicreadrequest, and returns a pin box containing an async future, that returns a byte vec
            fun: Box::new(move |req| {
                let thread_tx = read_tx.clone();
                async move {
                    let value = OS.as_bytes().to_vec();
                    println!("Read request {:?} with value {:x?}", &req, &value);
                    if thread_tx
                        .send(BluetoothMessage::PeerReadSsid)
                        .await
                        .is_err()
                    {
                        return Err(ReqError::Failed);
                    }
                    Ok(value)
                }
                .boxed()
            }),
            ..Default::default()
        }),
        write: Some(CharacteristicWrite {
            write: true,
            write_without_response: false, // TODO: remove?
            secure_write: true,
            method: CharacteristicWriteMethod::Fun(Box::new(move |new_value, req| {
                // let value = value_write.clone();
                let thread_tx = write_tx.clone();
                async move {
                    println!("Write request {:?} with value {:x?}", &req, &new_value);
                    let peer_os = String::from_utf8(new_value).expect("Peer OS was not UTF-8");
                    if thread_tx
                        .send(BluetoothMessage::PeerOS(peer_os))
                        .await
                        .is_err()
                    {
                        return Err(ReqError::Failed);
                    }
                    Ok(())
                }
                .boxed()
            })),
            ..Default::default()
        }),
        // notify: Some(CharacteristicNotify {
        //     notify: true,
        //     method: CharacteristicNotifyMethod::Fun(Box::new(move |mut notifier| {
        //         // let value = value_notify.clone();
        //         async move {
        //             tokio::spawn(async move {
        //                 println!(
        //                     "Notification session start with confirming={:?}",
        //                     notifier.confirming()
        //                 );
        //                 loop {
        //                     {
        //                         let mut value = value.lock().await;
        //                         println!("Notifying with value {:x?}", &*value);
        //                         if let Err(err) = notifier.notify(value.to_vec()).await {
        //                             println!("Notification error: {}", &err);
        //                             break;
        //                         }
        //                         println!("Decrementing each element by one");
        //                         for v in &mut *value {
        //                             *v = v.saturating_sub(1);
        //                         }
        //                     }
        //                     sleep(Duration::from_secs(5)).await;
        //                 }
        //                 println!("Notification session stop");
        //             });
        //         }
        //         .boxed()
        //     })),
        //     ..Default::default()
        // }),
        ..Default::default()
    }
}

fn get_ssid_characteristic(tx: mpsc::Sender<BluetoothMessage>, ssid: String) -> Characteristic {
    let read_tx = tx.clone();
    let write_tx = tx.clone();
    Characteristic {
        uuid: Uuid::parse_str(SSID_CHARACTERISTIC_UUID).unwrap(),
        read: Some(CharacteristicRead {
            read: true,
            secure_read: true,
            fun: Box::new(move |req| {
                let ssid = ssid.clone();
                let thread_tx = read_tx.clone();
                async move {
                    let value = ssid.as_bytes().to_vec();
                    println!("Read request {:?} with value {:x?}", &req, &value);
                    if thread_tx
                        .send(BluetoothMessage::PeerReadSsid)
                        .await
                        .is_err()
                    {
                        return Err(ReqError::Failed);
                    }
                    Ok(value)
                }
                .boxed()
            }),
            ..Default::default()
        }),
        write: Some(CharacteristicWrite {
            write: true,
            write_without_response: false,
            secure_write: true,
            method: CharacteristicWriteMethod::Fun(Box::new(move |new_value, req| {
                let thread_tx = write_tx.clone();
                async move {
                    println!("Write request {:?} with value {:x?}", &req, &new_value);
                    let peer_ssid = String::from_utf8(new_value).expect("Peer OS was not UTF-8");
                    if thread_tx
                        .send(BluetoothMessage::SSID(peer_ssid))
                        .await
                        .is_err()
                    {
                        return Err(ReqError::Failed);
                    }
                    Ok(())
                }
                .boxed()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

fn get_password_characteristic(
    tx: mpsc::Sender<BluetoothMessage>,
    password: String,
) -> Characteristic {
    let read_tx = tx.clone();
    let write_tx = tx.clone();
    Characteristic {
        uuid: Uuid::parse_str(PASSWORD_CHARACTERISTIC_UUID).unwrap(),
        read: Some(CharacteristicRead {
            read: true,
            secure_read: true,
            fun: Box::new(move |req| {
                let password = password.clone();
                let thread_tx = read_tx.clone();
                async move {
                    let value = password.as_bytes().to_vec();
                    println!("Read request {:?} with value {:x?}", &req, &value);
                    if thread_tx
                        .send(BluetoothMessage::PeerReadPassword)
                        .await
                        .is_err()
                    {
                        return Err(ReqError::Failed);
                    }
                    Ok(value)
                }
                .boxed()
            }),
            ..Default::default()
        }),
        write: Some(CharacteristicWrite {
            write: true,
            write_without_response: false,
            secure_write: true,
            method: CharacteristicWriteMethod::Fun(Box::new(move |new_value, req| {
                let thread_tx = write_tx.clone();
                async move {
                    println!("Write request {:?} with value {:x?}", &req, &new_value);
                    let peer_password =
                        String::from_utf8(new_value).expect("Peer OS was not UTF-8");
                    if thread_tx
                        .send(BluetoothMessage::Password(peer_password))
                        .await
                        .is_err()
                    {
                        return Err(ReqError::Failed);
                    }
                    Ok(())
                }
                .boxed()
            })),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(crate) async fn advertise(
    tx: mpsc::Sender<BluetoothMessage>,
    ssid: &str,
    password: &str,
) -> bluer::Result<(ApplicationHandle, AdvertisementHandle)> {
    let service_uuid = Uuid::parse_str(SERVICE_UUID).unwrap();
    let session = bluer::Session::new().await?;
    let adapter = session.default_adapter().await?;
    adapter.set_powered(true).await?;

    println!(
        "Advertising on Bluetooth adapter {} with address {}",
        adapter.name(),
        adapter.address().await?
    );
    let le_advertisement = Advertisement {
        service_uuids: vec![service_uuid].into_iter().collect(),
        discoverable: Some(true),
        local_name: Some("Flying Carpet".to_string()),
        ..Default::default()
    };
    let adv_handle = adapter.advertise(le_advertisement).await?;

    println!(
        "Serving GATT service on Bluetooth adapter {}",
        adapter.name()
    );
    let app = Application {
        services: vec![Service {
            uuid: service_uuid,
            primary: true,
            characteristics: vec![
                get_os_characteristic(tx.clone()),
                get_ssid_characteristic(tx.clone(), ssid.to_string()),
                get_password_characteristic(tx, password.to_string()),
            ],
            ..Default::default()
        }],
        ..Default::default()
    };
    let app_handle = adapter.serve_gatt_application(app).await?;
    Ok((app_handle, adv_handle))
}
