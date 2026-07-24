use crate::{fc_error, FCError, InterfaceInfo, Mode, Peer, PeerResource, WiFiInterface, UI};
use regex::Regex;
use std::env::current_exe;
use std::ffi::{c_void, CString};
use std::os::windows::process::CommandExt;
use std::sync::mpsc;
use std::time::Duration;
use std::{process, thread};
use wifidirect_legacy_ap::WlanHostedNetworkHelper;
use windows::core::{GUID, HSTRING, PCSTR, PCWSTR, PSTR};
use windows::Win32::Foundation::{GetLastError, ERROR_SUCCESS, HANDLE, WIN32_ERROR};
use windows::Win32::NetworkManagement::IpHelper;
use windows::Win32::NetworkManagement::WiFi::{
    self, WLAN_INTERFACE_INFO, WLAN_INTERFACE_INFO_LIST,
};
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
) -> Result<PeerResource, FCError> {
    let hosting = is_hosting(&peer, &mode);
    if hosting {
        ensure_firewall_rules(ui).await?;

        // start hotspot
        let hosted_network = start_wifi_direct(&ssid, &password, ui)?;
        Ok(PeerResource::WindowsHotspot(hosted_network))
    } else {
        let guid = parse_interface_guid(&interface)?;
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
        ))
    }
}

fn parse_interface_guid(interface: &WiFiInterface) -> Result<GUID, FCError> {
    Ok(GUID::from_u128(interface_guid_u128(interface)?))
}

// The GUID string stored in WiFiInterface is the adapter GUID (the same GUID the WLAN
// API reports as InterfaceGuid), formatted as a base-10 u128.
fn interface_guid_u128(interface: &WiFiInterface) -> Result<u128, FCError> {
    u128::from_str_radix(&interface.1, 10).map_err(|e| FCError {
        message: format!("Invalid interface GUID '{}': {}", interface.1, e),
    })
}

// Parses an adapter's AdapterName (a braced GUID string like "{4B0A...}") into the
// same u128 form GUID::to_u128() produces, so entries from GetAdaptersAddresses and
// from the WLAN API share one matching key. Note that this is the *adapter* GUID:
// NetworkGuid identifies the network profile instead, which two adapters on the same
// network can share, so it can't be used to identify an adapter.
unsafe fn adapter_guid_u128(adapter: *mut IpHelper::IP_ADAPTER_ADDRESSES_LH) -> Option<u128> {
    let name = (*adapter).AdapterName;
    if name.is_null() {
        return None;
    }
    let name = std::ffi::CStr::from_ptr(name.0 as *const _).to_string_lossy();
    let hex: String = name.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 32 {
        return None;
    }
    u128::from_str_radix(&hex, 16).ok()
}

// First IPv4 address on the adapter that could carry a transfer (loopback and
// link-local/APIPA addresses mean there's no usable network).
unsafe fn first_usable_ipv4(
    adapter: *mut IpHelper::IP_ADAPTER_ADDRESSES_LH,
) -> Option<std::net::Ipv4Addr> {
    let mut unicast = (*adapter).FirstUnicastAddress;
    while !unicast.is_null() {
        let address = (*unicast).Address;
        if !address.lpSockaddr.is_null() {
            let sa_data = (*address.lpSockaddr).sa_data;
            let mut octets = [0u8; 4];
            for i in 2..=5 {
                octets[i - 2] = sa_data[i] as u8;
            }
            let ip = std::net::Ipv4Addr::from(octets);
            if !ip.is_loopback() && !ip.is_link_local() && !ip.is_unspecified() {
                return Some(ip);
            }
        }
        unicast = (*unicast).Next;
    }
    None
}

// shown when the adapter/driver can't host a hotspot (#115): point the user at the
// fallback that doesn't require one
const WIFI_DIRECT_FAILURE_HINT: &str = "This usually means your Wi-Fi adapter or its driver doesn't support hosting a hotspot. Try Shared Network mode instead: connect both devices to the same network and select \"Shared Network\" on each.";

fn start_wifi_direct<T: UI>(ssid: &str, password: &str, ui: &T) -> Result<WindowsHotspot, FCError> {
    // Make channels to receive messages from Windows Runtime
    let (message_tx, message_rx) = mpsc::channel::<String>();
    let (success_tx, success_rx) = mpsc::channel::<bool>();
    // TODO: we should be able to use ? here, need to bump wifidirect-legacy-ap's windows-rs version?
    let hosted_network = match WlanHostedNetworkHelper::new(ssid, password, message_tx, success_tx)
    {
        Ok(hn) => hn,
        Err(e) => Err(FCError {
            message: format!("{} {}", e, WIFI_DIRECT_FAILURE_HINT),
        })?,
    };

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
        Err(FCError {
            message: format!("Failed to start WiFi Direct AP. {}", WIFI_DIRECT_FAILURE_HINT),
        })
    }
}

pub fn stop_hotspot(
    peer_resource: Option<&PeerResource>,
    _ssid: Option<&str>,
) -> Result<String, FCError> {
    // if we're joining, not hosting, we don't need to do anything here. and on windows PeerResource should never be LinuxHotspot.
    match peer_resource {
        // TODO: we should be able to use ? here, need to bump wifidirect-legacy-ap's windows-rs version?
        Some(PeerResource::WindowsHotspot(hotspot)) => {
            if let Err(e) = hotspot._inner.stop() {
                Err(FCError {
                    message: e.to_string(),
                })?;
            }
        }
        Some(PeerResource::WifiClient(_)) => {
            // TODO: delete network? no, letting the hotspot disappear is better because the client automatically goes back to its previous network?
        }
        _ => (),
    }
    Ok("Hotspot stopped".to_string())
}

fn run_shell_execute(
    program: &str,
    parameters: Option<&str>,
    as_admin: bool,
) -> Result<(), FCError> {
    let mode = rust_to_pcstr(if as_admin { "runas" } else { "open" });
    let program = rust_to_pcstr(program);
    let parameters = match parameters {
        Some(p) => rust_to_pcstr(p),
        None => PCSTR::null(),
    };
    unsafe {
        CoInitialize(None).unwrap();
        let res = ShellExecuteA(GetDesktopWindow(), mode, program, parameters, None, SW_HIDE);
        let res = res.0 as isize;
        if res < 32 {
            let error_message = get_windows_error(GetLastError().0)?;
            fc_error(&error_message)?;
        }
    }
    Ok(())
}

/// Get local IPv4 address on the specified interface (works for WiFi or wired)
pub fn get_local_ip(interface: &WiFiInterface) -> Result<std::net::Ipv4Addr, FCError> {
    let target_guid = interface_guid_u128(interface)?;

    let working_buffer_size = 15_000;
    let family = 2; // IPv4
    let flags = IpHelper::GAA_FLAG_INCLUDE_PREFIX;
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
            fc_error(&format!(
                "Could not get adapter addresses: {}",
                get_windows_error(res)?
            ))?;
        }

        while !pip_ip_adapter_addresses_lh.is_null() {
            if adapter_guid_u128(pip_ip_adapter_addresses_lh) == Some(target_guid) {
                if let Some(ip) = first_usable_ipv4(pip_ip_adapter_addresses_lh) {
                    return Ok(ip);
                }
            }

            pip_ip_adapter_addresses_lh = (*pip_ip_adapter_addresses_lh).Next;
        }
    }

    // Fallback: try to get any WiFi interface IP
    get_any_wifi_ip()
}

/// Get the subnet prefix length (e.g. 24 for /24) on the specified interface
pub fn get_prefix_length(interface: &WiFiInterface) -> Result<u8, FCError> {
    let target_guid = interface_guid_u128(interface)?;

    let working_buffer_size = 15_000;
    let family = 2; // IPv4
    let flags = IpHelper::GAA_FLAG_INCLUDE_PREFIX;
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
            fc_error(&format!(
                "Could not get adapter addresses: {}",
                get_windows_error(res)?
            ))?;
        }

        while !pip_ip_adapter_addresses_lh.is_null() {
            if adapter_guid_u128(pip_ip_adapter_addresses_lh) == Some(target_guid) {
                let unicast = (*pip_ip_adapter_addresses_lh).FirstUnicastAddress;
                if !unicast.is_null() {
                    return Ok((*unicast).OnLinkPrefixLength);
                }
            }
            pip_ip_adapter_addresses_lh = (*pip_ip_adapter_addresses_lh).Next;
        }
    }

    // Default to /24 if we can't determine
    Ok(24)
}

fn get_any_wifi_ip() -> Result<std::net::Ipv4Addr, FCError> {
    let working_buffer_size = 15_000;
    let family = 2; // IPv4
    let flags = IpHelper::GAA_FLAG_INCLUDE_PREFIX;
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
            fc_error(&format!(
                "Could not get adapter addresses: {}",
                get_windows_error(res)?
            ))?;
        }

        while !pip_ip_adapter_addresses_lh.is_null() {
            if (*pip_ip_adapter_addresses_lh).IfType == IpHelper::IF_TYPE_IEEE80211
                || (*pip_ip_adapter_addresses_lh).IfType == IpHelper::IF_TYPE_ETHERNET_CSMACD
            {
                let unicast = (*pip_ip_adapter_addresses_lh).FirstUnicastAddress;
                if !unicast.is_null() {
                    let address = (*unicast).Address;
                    let sa_data = (*address.lpSockaddr).sa_data;

                    let mut octets = [0u8; 4];
                    for i in 2..=5 {
                        octets[i - 2] = sa_data[i] as u8;
                    }

                    let ip = std::net::Ipv4Addr::from(octets);
                    // Skip loopback and link-local addresses
                    if !ip.is_loopback() && !ip.is_link_local() {
                        return Ok(ip);
                    }
                }
            }
            pip_ip_adapter_addresses_lh = (*pip_ip_adapter_addresses_lh).Next;
        }
    }

    fc_error("No network interface with IPv4 address found")?;
    unreachable!()
}

/// Check if interface has an active network connection
pub fn has_network_connection(interface: &WiFiInterface) -> Result<bool, FCError> {
    // Try to get an IP address for the interface - if we can, it's connected
    match get_local_ip(interface) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

/// WiFi and Ethernet interfaces that are up and have a usable IPv4 address, for shared
/// network mode (which works over wired connections too, unlike hotspot mode).
/// Interfaces without a network are omitted — they can't work in shared mode and only
/// clutter the chooser (unplugged adapters, hidden WiFi-Direct virtual adapters,
/// Bluetooth PAN). Interfaces holding a default route are listed first so the UI can
/// preselect the one most likely to be right.
pub fn get_connected_interfaces() -> Result<Vec<InterfaceInfo>, FCError> {
    let working_buffer_size = 15_000;
    let family = 2; // IPv4
    let flags = IpHelper::GAA_FLAG_INCLUDE_PREFIX | IpHelper::GAA_FLAG_INCLUDE_GATEWAYS;
    let mut ip_adapter_addresses_lh = vec![0u8; working_buffer_size];
    let mut pip_ip_adapter_addresses_lh =
        (ip_adapter_addresses_lh.as_mut_ptr()) as *mut IpHelper::IP_ADAPTER_ADDRESSES_LH;
    let mut size = working_buffer_size as u32;

    let mut with_gateway = Vec::new();
    let mut without_gateway = Vec::new();

    unsafe {
        let res = IpHelper::GetAdaptersAddresses(
            family,
            flags,
            None,
            Some(pip_ip_adapter_addresses_lh),
            &mut size,
        );
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            fc_error(&format!(
                "Could not get adapter addresses: {}",
                get_windows_error(res)?
            ))?;
        }

        while !pip_ip_adapter_addresses_lh.is_null() {
            let adapter = pip_ip_adapter_addresses_lh;
            pip_ip_adapter_addresses_lh = (*adapter).Next;

            if (*adapter).IfType != IpHelper::IF_TYPE_IEEE80211
                && (*adapter).IfType != IpHelper::IF_TYPE_ETHERNET_CSMACD
            {
                continue;
            }
            // IF_OPER_STATUS is a newtype over i32; IfOperStatusUp == 1
            if (*adapter).OperStatus.0 != 1 {
                continue;
            }
            let (guid, ip) = match (adapter_guid_u128(adapter), first_usable_ipv4(adapter)) {
                (Some(guid), Some(ip)) => (guid, ip),
                _ => continue,
            };

            let name = String::from_utf16_lossy(&(*adapter).FriendlyName.as_wide())
                .trim_matches(char::from(0))
                .to_string();
            let info = InterfaceInfo {
                name,
                guid: format!("{}", guid),
                ip: Some(ip.to_string()),
            };
            if (*adapter).FirstGatewayAddress.is_null() {
                without_gateway.push(info);
            } else {
                with_gateway.push(info);
            }
        }
    }

    with_gateway.append(&mut without_gateway);
    Ok(with_gateway)
}

// Looks up an adapter's friendly name (as shown in Windows' Network Connections panel)
// and usable IPv4 by adapter GUID. Used to label WLAN interfaces, whose WLAN-API name
// is the hardware description.
fn adapter_display_info(guid: u128) -> (Option<String>, Option<std::net::Ipv4Addr>) {
    let working_buffer_size = 15_000;
    let family = 2; // IPv4
    let flags = IpHelper::GAA_FLAG_INCLUDE_PREFIX;
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
            return (None, None);
        }

        while !pip_ip_adapter_addresses_lh.is_null() {
            if adapter_guid_u128(pip_ip_adapter_addresses_lh) == Some(guid) {
                let name = String::from_utf16_lossy(
                    &(*pip_ip_adapter_addresses_lh).FriendlyName.as_wide(),
                )
                .trim_matches(char::from(0))
                .to_string();
                let ip = first_usable_ipv4(pip_ip_adapter_addresses_lh);
                return (Some(name), ip);
            }
            pip_ip_adapter_addresses_lh = (*pip_ip_adapter_addresses_lh).Next;
        }
    }

    (None, None)
}

// returns Ok(Some(gateway)) if gateway found, Ok(None) if no gateway found but no error, and Err otherwise.
fn find_gateway() -> Result<Option<String>, FCError> {
    let working_buffer_size = 15_000;
    let family = 2; // IPv4
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
            fc_error(&format!(
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

                    // for some reason after the windows-rs version upgrade, sa_data were signed bytes
                    // and there were negative numbers in the ip address, so have to convert to u8
                    let mut unsigned_octets = [0u8; 4];
                    for i in 2..=5 {
                        unsigned_octets[i - 2] = sa_data[i] as u8;
                    }

                    // TODO: do this properly? https://stackoverflow.com/questions/1276294/getting-ipv4-address-from-a-sockaddr-structure
                    let gateway = format!(
                        "{}.{}.{}.{}",
                        unsigned_octets[0],
                        unsigned_octets[1],
                        unsigned_octets[2],
                        unsigned_octets[3]
                    );
                    return Ok(Some(gateway));
                }
            }
            pip_ip_adapter_addresses_lh = (*pip_ip_adapter_addresses_lh).Next;
        }
    }
    Ok(None)
}

// This is a hacky way to get information on all interfaces from Windows,
// not just the one that windows-rs's WLAN_INTERFACE_INFO_LIST gives you
unsafe fn wlan_enum_multiple_interfaces(
    client_handle: HANDLE,
    p_interface_list: *mut *mut WLAN_INTERFACE_INFO_LIST,
) -> Result<Vec<WLAN_INTERFACE_INFO>, FCError> {
    let res = WiFi::WlanEnumInterfaces(client_handle, None, p_interface_list);
    if WIN32_ERROR(res) != ERROR_SUCCESS {
        let err = format!(
            "Error enumerating WiFi interfaces: {}",
            get_windows_error(res)?
        );
        WiFi::WlanCloseHandle(client_handle, None);
        fc_error(&err)?;
    }
    let interfaces = std::slice::from_raw_parts(
        &(**p_interface_list).InterfaceInfo[0],
        (**p_interface_list).dwNumberOfItems as usize,
    );
    Ok(interfaces.to_vec())
}

pub fn get_wifi_interfaces() -> Result<Vec<InterfaceInfo>, FCError> {
    unsafe {
        // get client handle
        let mut client_handle = HANDLE::default();
        let mut negotiated_version = 0;
        let res = WiFi::WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle);
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            fc_error(&format!("open handle error: {}", get_windows_error(res)?))?;
        }
        // find wifi interface
        let mut interface_list = WiFi::WLAN_INTERFACE_INFO_LIST::default();
        let mut p_interface_list: *mut WiFi::WLAN_INTERFACE_INFO_LIST = &mut interface_list;

        let wlan_interfaces = wlan_enum_multiple_interfaces(client_handle, &mut p_interface_list)?;
        let mut interfaces: Vec<InterfaceInfo> = vec![];
        for wlan_interface in wlan_interfaces {
            let description = String::from_utf16_lossy(&wlan_interface.strInterfaceDescription)
                .trim_matches(char::from(0))
                .to_string();
            let guid = wlan_interface.InterfaceGuid.to_u128();
            // label with the friendly name from the adapter list ("Wi-Fi") rather than
            // the WLAN API's hardware description, plus the IP if it has a network.
            // no IP is fine here: hosting a hotspot doesn't need a connection.
            let (friendly_name, ip) = adapter_display_info(guid);
            interfaces.push(InterfaceInfo {
                name: friendly_name.unwrap_or(description),
                // store u128 GUID formatted as string because javascript can't handle 128-bit numbers
                guid: format!("{}", guid),
                ip: ip.map(|ip| ip.to_string()),
            });
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
) -> Result<(), FCError> {
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
        fc_error(&format!(
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

// Escapes a string for interpolation into WLAN profile XML. The SSID and password can
// come from the peer (e.g. an Android hotspot host), so they must not be able to
// inject profile elements.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn join_hotspot(ssid: &str, password: &str, guid: &GUID) -> Result<bool, FCError> {
    let mut client_handle = HANDLE::default();

    // 802.11 SSIDs are at most 32 bytes; enforce that before building the fixed-size
    // DOT11_SSID buffer below (which would otherwise panic on a hostile value)
    if ssid.len() > 32 {
        fc_error(&format!("SSID from peer is too long: {}", ssid))?;
    }
    let ssid_escaped = xml_escape(ssid);
    let password_escaped = xml_escape(password);

    let xml = "<?xml version=\"1.0\"?>\r\n".to_string()
        + "<WLANProfile xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v1\">\r\n"
        + "	<name>"
        + &ssid_escaped
        + "</name>\r\n"
        + "	<SSIDConfig>\r\n"
        + "		<SSID>\r\n"
        + "			<name>"
        + &ssid_escaped
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
        + &password_escaped
        + "</keyMaterial>\r\n"
        + "			</sharedKey>\r\n"
        + "		</security>\r\n"
        + "	</MSM>\r\n"
        + "	<MacRandomization xmlns=\"http://www.microsoft.com/networking/WLAN/profile/v3\">\r\n"
        + "		<enableRandomization>false</enableRandomization>\r\n"
        + "	</MacRandomization>\r\n"
        + "</WLANProfile>";
    let xml_hstring = HSTRING::from(xml);
    let str_profile = PCWSTR::from_raw(xml_hstring.as_ptr());

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
        strProfile: str_profile,
        pDot11Ssid: &mut dot11_ssid,
        pDesiredBssidList: std::ptr::null_mut(),
        dot11BssType: WiFi::dot11_BSS_type_any,
        dwFlags: 0,
    };
    unsafe {
        let mut negotiated_version = 0;
        let mut res = WiFi::WlanOpenHandle(2, None, &mut negotiated_version, &mut client_handle);
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            fc_error(&format!("open handle error: {}", get_windows_error(res)?))?;
        }

        let (tx, rx) = mpsc::channel();
        register_for_hotspot_connected_callback(tx.clone(), client_handle)?;

        res = WiFi::WlanConnect(client_handle, guid, &parameters, None);
        if WIN32_ERROR(res) != ERROR_SUCCESS {
            unregister_hotspot_callback(client_handle);
            WiFi::WlanCloseHandle(client_handle, None);
            fc_error(&format!("Connect error: {}", get_windows_error(res)?))?
        }

        let hotspot_started = rx.recv()?;
        unregister_hotspot_callback(client_handle);
        WiFi::WlanCloseHandle(client_handle, None);
        Ok(hotspot_started)
    }
}

/// Makes sure the inbound TCP and UDP rules for port 3290 exist, prompting for UAC if not.
/// TCP is needed whenever this machine is the TCP server (hotspot host, or receiver in
/// shared network mode); UDP is needed for shared network discovery announcements.
pub async fn ensure_firewall_rules<T: UI>(ui: &T) -> Result<(), FCError> {
    let path = current_exe()?;
    let path_string = path.to_string_lossy().to_string();
    let file_name = path
        .file_name()
        .expect("Error: couldn't convert path to string.")
        .to_string_lossy()
        .to_string();
    let udp_rule_name = format!("{} UDP", file_name);
    // The UDP rule was introduced with shared network mode, so it can be missing on
    // machines that already have the TCP rule from an earlier version.
    let need_tcp = !check_for_firewall_rule(&file_name, &path_string)?;
    let need_udp = !check_for_firewall_rule(&udp_rule_name, &path_string)?;
    if need_tcp || need_udp {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<Option<String>>(1);
        tokio::spawn(async move {
            let res = add_firewall_rule(need_tcp, need_udp);
            tx.send(res)
                .await
                .expect("couldn't send firewall UAC prompt response");
        });

        ui.output(
            "Waiting for permission to add firewall rule, please see UAC prompt in your taskbar.",
        );
        let res = rx.recv().await;
        let res = res.expect("couldn't unwrap value over channel");
        match res {
            Some(err_msg) => fc_error(&format!("couldn't add firewall rule. {}", err_msg))?,
            None => ui.output("Added firewall rule"),
        }

        // netsh runs in a separate elevated process, so confirm the rules actually landed
        for _ in 0..10 {
            if check_for_firewall_rule(&file_name, &path_string)?
                && check_for_firewall_rule(&udp_rule_name, &path_string)?
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        ui.output("Warning: could not verify the firewall rules were added. Incoming connections on port 3290 may be blocked.");
    } else {
        ui.output("Firewall rules already in place.");
    }
    Ok(())
}

/// Returns true only if an enabled rule with this name exists for *this* executable's path.
/// Matching by name alone isn't enough: an installed copy of Flying Carpet leaves rules with
/// the same name but a different program path, which don't allow this binary's traffic.
fn check_for_firewall_rule(rule_name: &str, program_path: &str) -> Result<bool, FCError> {
    // No embedded quotes: process::Command already passes this as one argument, and
    // netsh would otherwise search for a rule whose name literally contains quotes
    // (which made rule names with spaces, like the UDP rule, look permanently missing).
    let name = format!("name={}", rule_name);
    const CREATE_NO_WINDOW: u32 = 0x08000000; // https://learn.microsoft.com/en-us/windows/win32/procthread/process-creation-flags
    let mut command = process::Command::new("netsh");
    let command = command
        .args(vec![
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &name,
            "verbose",
        ])
        .creation_flags(CREATE_NO_WINDOW);
    match command.output() {
        Ok(output) => {
            let output_string = String::from_utf8_lossy(&output.stdout).to_string();
            let regex = Regex::new(r"Action:\s+Block")?;
            if regex.is_match(&output_string) {
                fc_error("a Windows Firewall rule is blocking Flying Carpet connections. Please delete or modify the rule to allow incoming connections on TCP port 3290.")?;
            }
            if !Regex::new(r"Enabled:\s+Yes")?.is_match(&output_string) {
                return Ok(false);
            }
            // verbose output includes a "Program:" line per rule; require our own path
            Ok(output_string
                .to_lowercase()
                .contains(&program_path.to_lowercase()))
        }
        Err(e) => Err(e)?,
    }
}

fn add_firewall_rule(add_tcp: bool, add_udp: bool) -> Option<String> {
    let path = &current_exe().expect("Error: couldn't get path to current executable.");
    let file_name = path
        .file_name()
        .expect("Error: couldn't convert path to string.")
        .to_string_lossy();

    let exe_path = path.to_string_lossy();

    // Each ShellExecute with the "runas" verb raises its own UAC prompt, so adding the TCP
    // and UDP rules as two separate netsh invocations made the user approve twice. Build
    // both commands and run them under a single elevated cmd.exe instead: one prompt.
    // The rule name and program path stay double-quoted, which also makes any cmd
    // metacharacter in the path (`&`, `|`, `^`) literal, so odd install paths are safe.
    let mut commands: Vec<String> = Vec::new();

    // TCP rule for file transfer
    if add_tcp {
        commands.push(format!(
            "netsh advfirewall firewall add rule name=\"{}\" dir=in action=allow program=\"{}\" enable=yes profile=any localport=3290 protocol=tcp",
            file_name, exe_path
        ));
    }

    // UDP rule for discovery (multicast + unicast)
    if add_udp {
        commands.push(format!(
            "netsh advfirewall firewall add rule name=\"{} UDP\" dir=in action=allow program=\"{}\" enable=yes profile=any localport=3290 protocol=udp",
            file_name, exe_path
        ));
    }

    if commands.is_empty() {
        return None;
    }

    // `&&` rather than `&`: if the first rule fails there's no point applying the second,
    // and the caller re-checks both rules afterward and reports if either is missing.
    let parameters = format!("/C {}", commands.join(" && "));
    if let Err(e) = run_shell_execute("cmd.exe", Some(&parameters), true) {
        return Some(e.to_string());
    }

    None
}

unsafe fn get_windows_error(err: u32) -> Result<String, FCError> {
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
        fc_error("Could not get error message from Windows")?;
    }
    Ok(error_message.to_string()?)
}

pub(crate) fn is_hosting(peer: &Peer, mode: &Mode) -> bool {
    // we're windows, so we always host if mac, linux, ios, or android.
    match peer {
        Peer::Android | Peer::IOS | Peer::Linux | Peer::MacOS => true,
        Peer::Windows => match mode {
            Mode::Send(_) => false,
            Mode::Receive(_) => true,
        },
    }
}

pub fn rust_to_pcstr(s: &str) -> PCSTR {
    PCSTR::from_raw(CString::new(s).unwrap().into_raw() as *const u8)
}

#[cfg(test)]
mod test {
    use crate::network::add_firewall_rule;
    use windows::core::GUID;

    // Manual test: fill in a real SSID and password below and run with
    // `cargo test join_hotspot -- --ignored`. As checked in (empty credentials),
    // Windows rejects the profile, so it's excluded from normal test runs.
    #[test]
    #[ignore]
    fn join_hotspot() {
        // put ssid and password here
        let interfaces = super::get_wifi_interfaces().expect("couldn't get wifi interfaces");
        let guid = u128::from_str_radix(&interfaces[0].guid, 10)
            .expect("couldn't get u128 guid from string");
        let guid = GUID::from_u128(guid);
        super::join_hotspot("", "", &guid).unwrap();
        // unsafe {
        //     std::thread::sleep(std::time::Duration::from_secs(10));
        //     super::delete_network("").unwrap();
        // }
    }

    // Manual test: adds real Windows Firewall rules via an elevated `netsh` (so it also
    // raises a UAC prompt) and never removes them. The rule is named after the *test
    // binary*, whose filename carries a build hash, so every rebuild that changes the
    // dependency graph leaves behind a fresh pair of TCP/UDP rules to clean up by hand.
    // Excluded from normal runs; run deliberately with
    // `cargo test check_for_firewall_rule -- --ignored`.
    #[test]
    #[ignore]
    fn check_for_firewall_rule() {
        let path = std::env::current_exe().unwrap();
        let path_string = path.to_string_lossy().to_string();
        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
        if !super::check_for_firewall_rule(&file_name, &path_string).unwrap() {
            add_firewall_rule(true, true);
        } else {
            println!("firewall rule present");
        }
        std::thread::sleep(std::time::Duration::from_secs(2));
        let rule_present = super::check_for_firewall_rule(&file_name, &path_string).unwrap();
        assert!(rule_present);
    }

    #[test]
    fn get_wifi_interfaces() {
        match crate::network::get_wifi_interfaces() {
            Ok(ifaces) => {
                for i in ifaces {
                    println!("{} {:?}", i.name, i.ip);
                }
            }
            Err(e) => println!("{}", e),
        }
    }
}
