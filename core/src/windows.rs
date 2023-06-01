use crate::utils::{run_command, rust_to_pcstr};
use crate::{Mode, Peer, PeerResource, WiFiInterface, UI};
use regex::Regex;
use std::env::current_exe;
use std::error::Error;
use std::ffi::c_void;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use wifidirect_legacy_ap::WlanHostedNetworkHelper;
use windows::core::{GUID, HSTRING, PCWSTR, PSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_SUCCESS, HANDLE, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper;
use windows::Win32::NetworkManagement::WiFi::{self, WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST};
use windows::Win32::Networking::WinSock;
use windows::Win32::System::Com::CoInitialize;
use windows::Win32::System::Diagnostics::Debug::{
    self, FORMAT_MESSAGE_FROM_SYSTEM, FORMAT_MESSAGE_IGNORE_INSERTS,
};
use windows::Win32::UI::Shell::ShellExecuteA;
use windows::Win32::UI::WindowsAndMessaging::{GetDesktopWindow, SW_HIDE};

pub struct WindowsHotspot {
    _inner: WlanHostedNetworkHelper,
}

pub async fn connect_to_peer<T: UI>(
    peer: Peer,
    mode: Mode,
    ssid: String,
    password: String,
    interface: WiFiInterface,
    ui: &T,
) -> Result<PeerResource, Box<dyn Error>> {
    let hosting = is_hosting(peer, mode);
    if hosting {
        if !check_for_firewall_rule()? {
            // open firewall
            let (tx, mut rx) = tokio::sync::mpsc::channel::<Option<String>>(1);
            tokio::spawn(async move {
                let res = add_firewall_rule();
                tx.send(res)
                    .await
                    .expect("couldn't send firewall UAC prompt response");
            });

            ui.output("Waiting for permission to add firewall rule, please see UAC prompt in your taskbar.");
            let res = rx.recv().await;
            let res = res.expect("couldn't unwrap value over channel");
            match res {
                Some(err_msg) => Err(format!("couldn't add firewall rule. {}", err_msg))?,
                None => ui.output("Added firewall rule"),
            }
        } else {
            ui.output("Firewall rule already in place.");
        }

        // start hotspot
        let hosted_network = match start_wifi_direct(&ssid, &password, ui) {
            Ok(ap) => ap,
            Err(e) => Err(e)?,
        };
        Ok(PeerResource::WindowsHotspot(hosted_network))
    } else {
        let guid =
            u128::from_str_radix(&interface.1, 10).expect("couldn't get u128 guid from string");
        let guid = GUID::from_u128(guid);
        loop {
            tokio::task::yield_now().await;
            ui.output("Trying to join hotspot...");
            if join_hotspot(&ssid, &password, &guid)? {
                ui.output(&format!("Connected to {}", ssid));
                break;
            }
            thread::sleep(Duration::from_secs(2));
        }
        let mut gateway = None;
        while gateway == None {
            tokio::task::yield_now().await;
            gateway = find_gateway()?;
            if let Some(g) = gateway.clone() {
                ui.output(&format!("WifiClient: {}", g));
            }
            thread::sleep(Duration::from_millis(200));
        }
        // expect is safe because gateway != None after while loop?
        // or is there a chance that cancelling during that .await could let this function complete?
        Ok(PeerResource::WifiClient(
            gateway.expect("Gateway == None when it shouldn't"),
            ssid,
        ))
    }
}

fn start_wifi_direct<T: UI>(
    ssid: &str,
    password: &str,
    ui: &T,
) -> Result<WindowsHotspot, Box<dyn Error>> {
    // Make channels to receive messages from Windows Runtime
    let (message_tx, message_rx) = mpsc::channel::<String>();
    let (success_tx, success_rx) = mpsc::channel::<bool>();
    let hosted_network = WlanHostedNetworkHelper::new(ssid, password, message_tx, success_tx)?;

    let thread_ui = ui.clone();

    std::thread::spawn(move || loop {
        let msg = match message_rx.recv() {
            Ok(m) => m,
            Err(_e) => {
                // thread_ui.output(&format!("WiFiDirect thread exiting: {}", _e));
                break;
            }
        };
        thread_ui.output(&msg);
    });

    let started = success_rx
        .recv()
        .expect("Could not receive whether WiFiDirect started");
    if started {
        Ok(WindowsHotspot {
            _inner: hosted_network,
        })
    } else {
        Err("Failed to start WiFi Direct AP".into())
    }
}

pub fn stop_hotspot(peer_resource: &PeerResource) -> Result<(), Box<dyn Error>> {
    // if we're joining, not hosting, we don't need to do anything here. and on windows PeerResource should never be LinuxHotspot.
    match peer_resource {
        PeerResource::WindowsHotspot(hotspot) => hotspot._inner.stop()?,
        PeerResource::WifiClient(_, _ssid) => {
            // delete network? no, letting the hotspot disappear is better because the client automatically goes back to its previous network
            // and this way we don't have to keep the previous ssid
        }
        _ => (),
    }
    Ok(())
}

fn run_shell_execute(
    program: &str,
    parameters: Option<&str>,
    as_admin: bool,
) -> Result<(), Box<dyn Error>> {
    let mode = rust_to_pcstr(if as_admin { "runas" } else { "open" });
    let program = rust_to_pcstr(program);
    let parameters = match parameters {
        Some(p) => Some(rust_to_pcstr(p)),
        None => None,
    };
    unsafe {
        CoInitialize(None).unwrap();
        let res = ShellExecuteA(GetDesktopWindow(), mode, program, parameters, None, SW_HIDE);
        if res.0 < 32 {
            let error_message = get_windows_error(GetLastError().0)?;
            Err(error_message)?;
        }
    }
    Ok(())
}

// returns Ok(Some(gateway)) if gateway found, Ok(None) if no gateway found but no error, and Err otherwise.
fn find_gateway() -> Result<Option<String>, Box<dyn Error>> {
    let working_buffer_size = 15_000;
    let family = WinSock::ADDRESS_FAMILY(2); // IPv4
    let flags = IpHelper::GAA_FLAG_INCLUDE_GATEWAYS;
    let mut ip_adapter_addresses_lh = vec![0u8; working_buffer_size];
    let mut pip_ip_adapter_addresses_lh =
        (ip_adapter_addresses_lh.as_mut_ptr()) as *mut IpHelper::IP_ADAPTER_ADDRESSES_LH;
    let mut size = working_buffer_size as u32;

    unsafe {
        let res = IpHelper::GetAdaptersAddresses(
            family,
            flags,
            None,
            Some(pip_ip_adapter_addresses_lh),
            &mut size,
        );
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            Err(format!(
                "Could not get adapter addresses: {}",
                get_windows_error(res)?
            ))?;
        }
        while !pip_ip_adapter_addresses_lh.is_null() {
            if (*pip_ip_adapter_addresses_lh).IfType == IpHelper::IF_TYPE_IEEE80211 {
                let gateway = (*pip_ip_adapter_addresses_lh).FirstGatewayAddress;
                if !gateway.is_null() {
                    let address = (*gateway).Address;
                    let sa_data = (*address.lpSockaddr).sa_data;
                    // TODO: do this properly? https://stackoverflow.com/questions/1276294/getting-ipv4-address-from-a-sockaddr-structure
                    let gateway = format!(
                        "{}.{}.{}.{}",
                        sa_data[2].0, sa_data[3].0, sa_data[4].0, sa_data[5].0
                    );
                    return Ok(Some(gateway));
                }
            }
            pip_ip_adapter_addresses_lh = (*pip_ip_adapter_addresses_lh).Next;
        }
    }
    Ok(None)
}

unsafe fn wlan_enum_multiple_interfaces(client_handle: HANDLE, p_interface_list: *mut *mut WLAN_INTERFACE_INFO_LIST)
    -> Result<Vec<WLAN_INTERFACE_INFO>, Box<dyn Error>>
{
    let res = WiFi::WlanEnumInterfaces(client_handle, None, p_interface_list);
    if WIN32_ERROR(res) != ERROR_SUCCESS {
        let err = format!(
            "Error enumerating WiFi interfaces: {}",
            get_windows_error(res)?
        );
        WiFi::WlanCloseHandle(client_handle, None);
        Err(err)?;
    }
    let list = **p_interface_list;
    println!("num interfaces: {}", list.dwNumberOfItems);
    let interfaces = std::slice::from_raw_parts(&list.InterfaceInfo[0], list.dwNumberOfItems as usize);
    Ok(interfaces.to_vec())
}

pub fn get_wifi_interfaces() -> Result<Vec<WiFiInterface>, Box<dyn Error>> {
    unsafe {
        // get client handle
        let mut client_handle = HANDLE::default();
        let mut negotiated_version = 0;
        let res = WiFi::WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle);
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            Err(format!("open handle error: {}", get_windows_error(res)?))?;
        }
        // find wifi interface
        let mut interface_list = WiFi::WLAN_INTERFACE_INFO_LIST::default();
        let mut p_interface_list: *mut WiFi::WLAN_INTERFACE_INFO_LIST = &mut interface_list;

        let wlan_interfaces = wlan_enum_multiple_interfaces(client_handle, &mut p_interface_list)?;
        let mut interfaces: Vec<WiFiInterface> = vec![];
        for wlan_interface in wlan_interfaces {
            let name = String::from_utf16_lossy(&wlan_interface.strInterfaceDescription)
                .trim_matches(char::from(0))
                .to_string();
            let guid = wlan_interface.InterfaceGuid.to_u128();
            let guid = format!("{}", guid); // store u128 GUID formatted as string because javascript can't handle 128-bit numbers
            interfaces.push(WiFiInterface(name, guid));
        }
        WiFi::WlanFreeMemory(p_interface_list as *const c_void);
        WiFi::WlanCloseHandle(client_handle, None);
        Ok(interfaces)
    }
}

unsafe extern "system" fn wifi_status_callback(
    notification_data: *mut WiFi::L2_NOTIFICATION_DATA,
    context: *mut c_void,
) {
    if (*notification_data).NotificationCode
        == WiFi::wlan_notification_acm_connection_complete.0 as u32
    {
        // don't reconstruct the box and let it be dropped unless we have something to say on tx
        let tx = context as *mut mpsc::Sender<bool>;
        let tx = Box::from_raw(tx);
        // let tx = &mut *tx;
        let reason_code =
            (*notification_data).pData as *mut WiFi::WLAN_CONNECTION_NOTIFICATION_DATA;
        let reason_code = &mut *reason_code;
        // println!("reason code: {}", reason_code.wlanReasonCode);
        if reason_code.wlanReasonCode == WiFi::WLAN_REASON_CODE_SUCCESS {
            tx.send(true)
                .expect("Could not send on channel from WLAN_NOTIFICATION_CALLBACK");
        } else {
            tx.send(false)
                .expect("Could not send on channel from WLAN_NOTIFICATION_CALLBACK");
        }
    }
    // println!(
    //     "notification code: {}",
    //     (*notification_data).NotificationCode
    // );
}

unsafe fn register_for_hotspot_connected_callback(
    tx: mpsc::Sender<bool>,
    client_handle: HANDLE,
) -> Result<(), Box<dyn Error>> {
    // make orphaned with into_raw() and cast to *c_void
    // windows callback will reconstruct this box when it has something to say
    // TODO: should it be Box<Mutex<Sender<String>>> because Sender is !Sync?
    // or is it ok because this function takes ownership of tx and we know it will only be used in callback?
    let callback_tx = Box::new(tx);
    let callback_tx = Box::into_raw(callback_tx);
    let callback_tx = callback_tx as *mut c_void;

    let res = WiFi::WlanRegisterNotification(
        client_handle,
        WiFi::WLAN_NOTIFICATION_SOURCE_ACM,
        true,
        Some(wifi_status_callback),
        Some(callback_tx),
        None,
        None,
    );
    if WIN32_ERROR(res) != ERROR_SUCCESS {
        Err(format!(
            "Error registering WLAN notification callback: {}",
            get_windows_error(res)?
        ))?;
    }
    Ok(())
}

unsafe fn unregister_hotspot_callback(client_handle: HANDLE) {
    let _res = WiFi::WlanRegisterNotification(
        client_handle,
        WiFi::WLAN_NOTIFICATION_SOURCE_NONE,
        true,
        None,
        None,
        None,
        None,
    );
    // if WIN32_ERROR(res) != ERROR_SUCCESS {
    //     println!("Could not unregister WLAN callback");
    // } else {
    //     println!("Unregistered hotspot callback");
    // }
    // don't really care if this failed, don't need to error handle here?
}

fn join_hotspot(
    ssid: &str,
    password: &str,
    guid: &GUID,
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut client_handle = HANDLE::default();

    let xml = "<?xml version=\"1.0\"?>\r\n".to_string()
        + "<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\r\n"
        + "	<name>"
        + ssid
        + "</name>\r\n"
        + "	<SSIDConfig>\r\n"
        + "		<SSID>\r\n"
        + "			<name>"
        + ssid
        + "</name>\r\n"
        + "		</SSID>\r\n"
        + "	</SSIDConfig>\r\n"
        + "	<connectionType>ESS</connectionType>\r\n"
        + "	<connectionMode>auto</connectionMode>\r\n"
        + "	<MSM>\r\n"
        + "		<security>\r\n"
        + "			<authEncryption>\r\n"
        + "				<authentication>WPA2PSK</authentication>\r\n"
        + "				<encryption>AES</encryption>\r\n"
        + "				<useOneX>false</useOneX>\r\n"
        + "			</authEncryption>\r\n"
        + "			<sharedKey>\r\n"
        + "				<keyType>passPhrase</keyType>\r\n"
        + "				<protected>false</protected>\r\n"
        + "				<keyMaterial>"
        + password
        + "</keyMaterial>\r\n"
        + "			</sharedKey>\r\n"
        + "		</security>\r\n"
        + "	</MSM>\r\n"
        + "	<MacRandomization xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v3\">\r\n"
        + "		<enableRandomization>false</enableRandomization>\r\n"
        + "	</MacRandomization>\r\n"
        + "</WLANProfile>";

    let str_profile = HSTRING::from(xml);
    let mut uc_ssid = [0u8; 32];
    let ssid_chars = ssid.as_bytes().to_vec();
    for i in 0..ssid_chars.len() {
        uc_ssid[i] = ssid_chars[i];
    }
    let mut dot11_ssid = WiFi::DOT11_SSID {
        uSSIDLength: ssid_chars.len() as u32,
        ucSSID: uc_ssid,
    };
    let parameters = WiFi::WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: WiFi::wlan_connection_mode_temporary_profile,
        strProfile: PCWSTR::from(&str_profile),
        pDot11Ssid: &mut dot11_ssid,
        pDesiredBssidList: std::ptr::null_mut(),
        dot11BssType: WiFi::dot11_BSS_type_any,
        dwFlags: 0,
    };
    unsafe {
        let mut negotiated_version = 0;
        let mut res = WiFi::WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle);
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            Err(format!("open handle error: {}", get_windows_error(res)?))?;
        }

        let (tx, rx) = mpsc::channel();
        register_for_hotspot_connected_callback(tx.clone(), client_handle)?;

        res = WiFi::WlanConnect(client_handle, guid, &parameters, None);
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            unregister_hotspot_callback(client_handle);
            WiFi::WlanCloseHandle(client_handle, None);
            Err(format!("Connect error: {}", get_windows_error(res)?))?
        }

        let hotspot_started = rx.recv()?;
        unregister_hotspot_callback(client_handle);
        WiFi::WlanCloseHandle(client_handle, None);
        Ok(hotspot_started)
    }
}

fn check_for_firewall_rule() -> Result<bool, Box<dyn Error>> {
    let path = &current_exe()?;
    let file_name = path
        .file_name()
        .expect("Error: couldn't convert path to string.")
        .to_string_lossy();
    let name = format!("name=\"{}\"", file_name);
    match run_command(
        "netsh",
        Some(vec!["advfirewall", "firewall", "show", "rule", &name]),
    ) {
        Ok(output) => {
            // if output contains enabled: true, return true
            let output_string = String::from_utf8_lossy(&output.stdout).to_string();
            let regex = Regex::new(r"Action:\s+Block")?;
            if regex.is_match(&output_string) {
                Err("a Windows Firewall rule is blocking Flying Carpet connections. Please delete or modify the rule to allow incoming connections on TCP port 3290.")?;
            }
            let regex = Regex::new(r"Enabled:\s+Yes")?;
            Ok(regex.is_match(&output_string))
        }
        Err(e) => Err(e)?,
    }
}

fn add_firewall_rule() -> Option<String> {
    let path = &current_exe().expect("Error: couldn't get path to current executable.");
    let file_name = path
        .file_name()
        .expect("Error: couldn't convert path to string.")
        .to_string_lossy();

    let program = "netsh";
    let parameters = "advfirewall firewall add rule name=\"".to_string()
        + &file_name
        + "\" dir=in action=allow program=\""
        + &path.to_string_lossy()
        + "\" enable=yes profile=any localport=3290 protocol=tcp";
    match run_shell_execute(program, Some(&parameters), true) {
        Ok(_) => None,
        Err(e) => Some(e.to_string()),
    }
}

unsafe fn get_windows_error(err: u32) -> Result<String, Box<dyn Error>> {
    let err = WIN32_ERROR(err);
    let msg_size = 1 << 10; // 1KB
    let mut buffer = vec![0u8; msg_size];
    let p_buffer: *mut u8 = &mut buffer[0];
    let error_message = PSTR::from_raw(p_buffer);
    let res = Debug::FormatMessageA(
        FORMAT_MESSAGE_FROM_SYSTEM | FORMAT_MESSAGE_IGNORE_INSERTS,
        None,
        err.0,
        0,
        error_message,
        msg_size as u32,
        None,
    );
    if res == 0 {
        Err("Could not get error message from Windows")?;
    }
    Ok(error_message.to_string()?)
}

fn is_hosting(peer: Peer, mode: Mode) -> bool {
    // we're windows, so we always host if mac, linux, ios, or android.
    match peer {
        Peer::Android | Peer::IOS | Peer::Linux | Peer::MacOS => true,
        Peer::Windows => match mode {
            Mode::Send(_) => false,
            Mode::Receive(_) => true,
        },
    }
}

#[cfg(test)]
mod test {
    use crate::network::add_firewall_rule;

    // #[test]
    // fn join_hotspot() {
    //     // put ssid and password here
    //     crate::network::join_hotspot("", "").unwrap();
    //     // unsafe {
    //     //     std::thread::sleep(std::time::Duration::from_secs(10));
    //     //     crate::network::delete_network("").unwrap();
    //     // }
    // }

    #[test]
    fn check_for_firewall_rule() {
        if !super::check_for_firewall_rule().unwrap() {
            add_firewall_rule();
        } else {
            println!("firewall rule present");
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
        let rule_present = super::check_for_firewall_rule().unwrap();
        assert!(rule_present);
    }

    #[test]
    fn get_wifi_interfaces() {
        match crate::network::get_wifi_interfaces() {
            Ok(ifaces) => {
                for i in ifaces {
                    println!("{:?}", i.0);
                }
            }
            Err(e) => println!("{}", e),
        }
    }
}
