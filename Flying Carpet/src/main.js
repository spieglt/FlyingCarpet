const { core, dialog, os } = window.__TAURI__;
import { QRCode } from './deps/qrcode.js'

let aboutButton;
let canUseBluetooth = false;
let usingBluetooth;
let bluetoothSwitch;
let sendFolderCheckbox;
let peerLabel;
let peerBox;
let outputBox;
let startButton;
let cancelButton;
let progressBar;
let appWindow;
let connectionModeLabel;
let connectionModeBox;

let selectedMode;
let selectedPeer;
let selectedFiles;
let selectedFolder;
let connectionMode = 'hotspot';

// save UI if user refreshes
window.onunload = () => {
  let uiState = {
    usingBluetooth: usingBluetooth,
    // canUseBluetooth:
    sendingFolder: sendFolderCheckbox.checked,
    selectedMode: selectedMode,
    selectedPeer: selectedPeer,
    selectedFiles: selectedFiles,
    selectedFolder: selectedFolder,
    output: outputBox.innerText,
    transferRunning: startButton.style.display === 'none',
    progressBarValue: progressBar.value,
    progressBarVisible: progressBar.style.display !== 'none',
    connectionMode: connectionMode,
  };
  let uiJSON = JSON.stringify(uiState);
  sessionStorage.setItem('pageState', uiJSON);
}

window.addEventListener('DOMContentLoaded', async () => {
  aboutButton = document.getElementById('aboutButton');
  peerLabel = document.getElementById('peerLabel');
  peerBox = document.getElementById('peerBox');
  outputBox = document.getElementById('outputBox');
  startButton = document.getElementById('startButton');
  cancelButton = document.getElementById('cancelButton');
  progressBar = document.getElementById('progressBar');
  bluetoothSwitch = document.getElementById('bluetoothSwitch');
  sendFolderCheckbox = document.getElementById('sendFolderCheckbox');
  connectionModeLabel = document.getElementById('connectionModeLabel');
  connectionModeBox = document.getElementById('connectionModeBox');

  appWindow = window.__TAURI__.window.getCurrentWindow();

  // check for bluetooth support
  let error = await core.invoke('check_support');
  if (error != null) {
    output(`Bluetooth initialization failed: ${error}. Disable the Bluetooth switch in Flying Carpet on the other device to run a transfer.`);
    bluetoothSwitch.disabled = true;
    bluetoothSwitch.checked = false;
    usingBluetooth = false;
    canUseBluetooth = false;
  } else {
    output('Bluetooth is supported.');
    bluetoothSwitch.disabled = false;
    bluetoothSwitch.checked = true;
    usingBluetooth = true;
    canUseBluetooth = true;
  }

  // about button
  aboutButton.onclick = () => {
    dialog.message(aboutMessage, { title: 'About Flying Carpet' });
  }

  // output handler
  await appWindow.listen('outputMsg', (event) => {
    output(event.payload.message);
  });

  // progress bar handlers
  await appWindow.listen('showProgressBar', (_event) => {
    progressBar.style.display = '';
  });
  await appWindow.listen('updateProgressBar', (event) => {
    progressBar.value = event.payload.value;
  });

  // enable UI when transfer finishes
  await appWindow.listen('enableUi', (_event) => {
    enableUi();
  });

  // show bluetooth PIN and allow user to choose whether to pair on windows
  await appWindow.listen('showPin', async (event) => {
    console.log(event);
    let choice = await dialog.ask(`Is this code displayed on the other device?\n\n${event.payload.message}`, { title: 'Confirm Bluetooth PIN', type: 'info' });
    console.log('choice:', choice);
    await core.invoke('user_bluetooth_pair', {
      choice: choice,
    });
    console.log('invoked user_bluetooth_pair');
  });

  // have Enter start/cancel transfer
  document.getElementById('mainContainer').addEventListener("keyup", event => {
    if (event.key !== "Enter") {
      return;
    }
    if (startButton.style.display != 'none' && !startButton.disabled) {
      startButton.click();
    }
    if (cancelButton.style.display != 'none') {
      cancelButton.click();
    }
    event.preventDefault();
  });

  // handle drag and drop
  await appWindow.onDragDropEvent(async event => {
    if (event.payload.type != 'drop') {
      return;
    }
    if (selectedMode === 'send') {
      selectedFiles = await core.invoke('expand_files', { paths: event.payload.paths });
      startTransfer(true);
    } else if (selectedMode === 'receive') {
      if (event.payload.length !== 1) {
        output('Error: if receiving, must drop only one destination folder.');
        return;
      }
      let is_dir = await core.invoke('is_dir', { path: event.payload[0] });
      if (is_dir) {
        selectedFolder = event.payload[0];
      } else {
        output('Error: if receiving, must select folder as destination.');
      }
      startTransfer(true);
    } else {
      output('Error: must select whether sending or receiving before dropping files or folder.');
    }
    checkStatus();
  });

  checkStatus();

  // rehydrate UI if user refreshed
  let uiState = JSON.parse(sessionStorage.getItem('pageState'));
  if (uiState) {
    usingBluetooth = uiState.usingBluetooth;
    bluetoothSwitch.checked = usingBluetooth;
    sendFolderCheckbox.checked = uiState.sendingFolder;
    selectedMode = uiState.selectedMode;
    if (selectedMode === 'send') {
      document.getElementById('sendButton').checked = true;
    } else if (selectedMode === 'receive') {
      document.getElementById('receiveButton').checked = true;
    }
    selectedPeer = uiState.selectedPeer;
    ['android', 'ios', 'linux', 'mac', 'windows'].forEach((os) => {
      let button = os + 'Button';
      if (selectedPeer === os) {
        document.getElementById(button).checked = true;
      }
    });
    // restore connection mode
    connectionMode = uiState.connectionMode || 'hotspot';
    if (connectionMode === 'shared_network') {
      document.getElementById('sharedNetworkButton').checked = true;
    } else {
      document.getElementById('hotspotButton').checked = true;
    }
    applyBluetoothAvailability();
    selectedFiles = uiState.selectedFiles;
    selectedFolder = uiState.selectedFolder;
    outputBox.innerText = uiState.output;
    progressBar.style.display = uiState.progressBarVisible ? '' : 'none';
    progressBar.value = uiState.progressBarValue;
    modeChange(selectedMode);
    if (uiState.transferRunning) {
      disableUi();
    }
    checkStatus();
  }
});

function output(msg) {
  outputBox.innerText += '\n' + msg;
  outputBox.scrollTop = outputBox.scrollHeight;
}

// in-page replacement for window.prompt(), whose title shows the webview origin.
// resolves to the entered string, or null if cancelled.
let showPrompt = (message) => {
  return new Promise((resolve) => {
    let overlay = document.getElementById('promptOverlay');
    let input = document.getElementById('promptInput');
    let okButton = document.getElementById('promptOk');
    let cancelButton = document.getElementById('promptCancel');
    document.getElementById('promptMessage').innerText = message;
    input.value = '';
    let finish = (value) => {
      overlay.style.display = 'none';
      okButton.onclick = null;
      cancelButton.onclick = null;
      input.onkeydown = null;
      resolve(value);
    };
    okButton.onclick = () => finish(input.value);
    cancelButton.onclick = () => finish(null);
    input.onkeydown = (event) => {
      event.stopPropagation();
      if (event.key === 'Enter') {
        finish(input.value);
      } else if (event.key === 'Escape') {
        finish(null);
      }
    };
    overlay.style.display = 'flex';
    input.focus();
  });
}

// same modal as showPrompt, but with a single-choice dropdown prepopulated with
// `options` instead of a text input. resolves with the selected index, or null on cancel.
let showSelect = (message, options) => {
  return new Promise((resolve) => {
    let overlay = document.getElementById('promptOverlay');
    let input = document.getElementById('promptInput');
    let select = document.getElementById('promptSelect');
    let okButton = document.getElementById('promptOk');
    let cancelButton = document.getElementById('promptCancel');
    document.getElementById('promptMessage').innerText = message;
    input.style.display = 'none';
    select.style.display = '';
    select.innerHTML = '';
    for (let i = 0; i < options.length; i++) {
      let option = document.createElement('option');
      option.value = i;
      option.innerText = options[i];
      select.appendChild(option);
    }
    let finish = (value) => {
      overlay.style.display = 'none';
      input.style.display = '';
      select.style.display = 'none';
      okButton.onclick = null;
      cancelButton.onclick = null;
      select.onkeydown = null;
      resolve(value);
    };
    okButton.onclick = () => finish(parseInt(select.value));
    cancelButton.onclick = () => finish(null);
    select.onkeydown = (event) => {
      event.stopPropagation();
      if (event.key === 'Enter') {
        finish(parseInt(select.value));
      } else if (event.key === 'Escape') {
        finish(null);
      }
    };
    overlay.style.display = 'flex';
    select.focus();
  });
}

function makeQRCode(str) {
  let elem = document.getElementById('qrcode');
  elem.innerHTML = '';
  new QRCode(elem, {
    text: str,
    width: 150,
    height: 150,
  });
}

async function startTransfer(filesSelected) {

  // the password is collected after files are chosen (below), so file selection isn't
  // gated on the other device having started and displayed its password yet.
  let password = null;

  // make sure we have a usable interface and prompt for which if more than one.
  // hotspot mode needs a wifi interface; shared network mode works over wired
  // (ethernet) interfaces too, so it uses the broader list. each interface is
  // {name, guid, ip}; label it with its IP (or lack of one) so the user can tell
  // connected interfaces apart, and pass [name, guid] to the backend (a WiFiInterface).
  let wifiInterface;
  let chosen;
  let interfaces = connectionMode === 'shared_network'
    ? await core.invoke('get_network_interfaces')
    : await core.invoke('get_wifi_interfaces');
  let interfaceLabel = (iface) => iface.ip ? `${iface.name} (${iface.ip})` : `${iface.name} (no network)`;
  // console.log('interfaces:', interfaces);
  switch (interfaces.length) {
    case 0:
      if (connectionMode === 'shared_network') {
        output('No connected network interfaces found. Connect to a network (WiFi or Ethernet) and try again.');
      } else {
        output('No WiFi interfaces found. Hotspot mode only works over WiFi.');
      }
      return;
    case 1:
      chosen = interfaces[0];
      output(`Using interface: ${interfaceLabel(chosen)}`);
      break;
    default: {
      let labels = interfaces.map(interfaceLabel);
      let choice = await showSelect('Select which network interface to use:', labels);
      if (choice === null) {
        output('Transfer cancelled.');
        return;
      }
      chosen = interfaces[choice];
      output(`Using interface: ${interfaceLabel(chosen)}`);
    }
  }
  wifiInterface = [chosen.name, chosen.guid];

  // if using shared network mode, check that we have a network connection
  if (connectionMode === 'shared_network') {
    let hasNetwork = await core.invoke('has_network_connection', { interface: wifiInterface });
    if (!hasNetwork) {
      output('No active network connection found. Shared Network mode requires both devices to be on the same local network. Please connect to a network or use Hotspot mode.');
      return;
    }
    output('Network connection detected. Using Shared Network mode.');
  }

  // get files or folder
  if (!filesSelected) {
    if (selectedMode == 'send') {
      if (sendFolderCheckbox.checked) {
        let folder = await dialog.open({
          multiple: false,
          directory: true,
        });
        if (!folder) {
          output('User cancelled.');
          return;
        }
        selectedFiles = await core.invoke('expand_files', { paths: [folder] });
      } else {
        await selectFiles();
        if (!selectedFiles) {
          output('User cancelled.');
          return;
        }
      }
    } else if (selectedMode == 'receive') {
      await selectFolder();
      if (!selectedFolder) {
        output('User cancelled.');
        return;
      }
    } else {
      output('Must select whether this device is sending or receiving.');
      return;
    }
  }
  
  // shared network sender: files are chosen, now get the password from the receiving device
  if (connectionMode === 'shared_network' && selectedMode === 'send') {
    let promptMessage = 'Enter the password displayed on the receiving device:';
    while (true) {
      password = await showPrompt(promptMessage);
      if (password === null) {
        output('Transfer cancelled.');
        return;
      }
      password = password.trim();
      if (password.length >= 8) {
        break;
      }
      promptMessage = 'Password must be at least 8 characters. Enter the password displayed on the receiving device:';
    }
  }

  // hotspot joiner: files are chosen, now get the password shown on the hosting device.
  // matches the shared-network prompt above so file selection is never gated on the
  // password (previously read from a box before the file dialog opened).
  if (await needPassword() && connectionMode !== 'shared_network') {
    let promptMessage = 'Enter the password displayed on the other device:';
    while (true) {
      password = await showPrompt(promptMessage);
      if (password === null) {
        output('Transfer cancelled.');
        return;
      }
      password = password.trim();
      if (password.length >= 8) {
        break;
      }
      promptMessage = 'Password must be at least 8 characters. Enter the password displayed on the other device:';
    }
  }

  // if we're generating the password (hosting in hotspot mode, or receiving in shared network mode),
  // and not using bluetooth (which exchanges the password automatically), generate and display it.
  if (!await needPassword() && !usingBluetooth) {
    password = await core.invoke('generate_password');
    if (connectionMode === 'shared_network') {
      // peer OS is unknown in shared network mode: show the password as text for desktop/Apple
      // senders and a QR code for Android senders.
      makeQRCode(password);
      output(`Password: ${password}`);
      output('Start the transfer on the sending device and enter this password when prompted (or scan the QR code on Android).');
      // not awaited: the transfer below must start without waiting for the dialog to be dismissed
      dialog.message(`Start the transfer on the sending device and enter this password when prompted (or scan the QR code on Android):\n\n${password}`, { title: 'Flying Carpet' });
    } else if (selectedPeer === 'ios' || selectedPeer === 'android') {
      output('\nStart the transfer on the other device and scan the QR code when prompted.');
      makeQRCode(password);
    } else {
      output(`Password: ${password}`);
      // not awaited: the transfer below must start without waiting for the dialog to be dismissed
      dialog.message(`Start the transfer on the other device and enter this password when prompted:\n\n${password}`, { title: 'Flying Carpet' });
    }
  }

  // disable UI
  disableUi();

  // kick off transfer
  await core.invoke('start_async', {
    mode: selectedMode,
    peer: selectedPeer,
    password: password,
    interface: wifiInterface,
    fileList: selectedFiles,
    receiveDir: selectedFolder,
    usingBluetooth: usingBluetooth,
    connectionMode: connectionMode,
    window: appWindow,
  });
}

async function cancelTransfer() {
  output(await core.invoke('cancel_transfer'));
}

let selectFiles = async () => {
  selectedFiles = await dialog.open({
    multiple: true,
    directory: false,
  });
  checkStatus();
}

let selectFolder = async () => {
  selectedFolder = await dialog.open({
    multiple: false,
    directory: true,
  });
  checkStatus();
}

let bluetoothChange = () => {
  usingBluetooth = bluetoothSwitch.checked;
  checkStatus();
}

let modeChange = async (button) => {
  startButton.innerText = button === 'receive' ? 'Select Folder' : 'Select Files';
  document.getElementById('sendFolderDiv').style.display = button === 'send' ? '' : 'none';
  selectedMode = button;
  checkStatus();
}

let peerChange = (button) => {
  selectedPeer = button;
  checkStatus();
}

let connectionModeChange = (mode) => {
  connectionMode = mode;
  applyBluetoothAvailability();
  checkStatus();
}

// Bluetooth is hotspot-only: in shared network mode the password is exchanged manually
// (receiver displays it, sender types it), so the switch is forced off and disabled.
let bluetoothCheckedBeforeShared = null; // remembers the switch state while in shared network mode
let applyBluetoothAvailability = () => {
  if (connectionMode === 'shared_network') {
    if (bluetoothCheckedBeforeShared === null) {
      bluetoothCheckedBeforeShared = bluetoothSwitch.checked;
    }
    bluetoothSwitch.checked = false;
    bluetoothSwitch.disabled = true;
    usingBluetooth = false;
  } else {
    bluetoothSwitch.disabled = !canUseBluetooth;
    if (bluetoothCheckedBeforeShared !== null) {
      bluetoothSwitch.checked = canUseBluetooth && bluetoothCheckedBeforeShared;
      bluetoothCheckedBeforeShared = null;
    }
    usingBluetooth = bluetoothSwitch.checked;
  }
}

let checkStatus = () => {
  if (connectionMode === 'shared_network' || usingBluetooth) {
    // Shared network: peer OS not needed (discovery handles it)
    // Bluetooth: peer OS not needed (exchanged over BLE)
    peerLabel.style.display = 'none';
    peerBox.style.display = 'none';
    startButton.disabled = !selectedMode;
  } else {
    peerLabel.style.display = '';
    peerBox.style.display = '';
    startButton.disabled = !(selectedMode && selectedPeer);
  }
}

let needPassword = async () => {
  // Shared network: receiver generates password, sender enters it (consistent with hotspot
  // same-platform convention). Bluetooth is never used in shared network mode.
  if (connectionMode === 'shared_network') {
    return selectedMode === 'send';
  }
  if (usingBluetooth) {
    return false;
  }
  // if linux, joining windows, hosting mac/ios/android or linux if receiving.
  // if windows, always hosting unless windows and sending.
  let showPassword;
  console.log('os:', os.type());
  switch (await os.type()) {
    case 'linux':
      showPassword = selectedPeer === 'windows' || (selectedPeer === 'linux' && selectedMode === 'send');
      break;
    case 'windows':
      showPassword = selectedPeer === 'windows' && selectedMode === 'send';
      break;
    default:
      alert('Error in needPassword()');
  }
  return showPassword;
}

let enableUi = async () => {
  // show start button
  startButton.style.display = '';
  // hide cancel button
  cancelButton.style.display = 'none';
  // enable bluetooth switch (stays disabled in shared network mode)
  applyBluetoothAvailability();
  // enable send folder box
  document.getElementById('sendFolderCheckbox').disabled = false;
  // enable radio buttons, file/folder selection buttons
  let radioButtons = ['sendButton', 'receiveButton', 'androidButton', 'iosButton', 'linuxButton', 'macButton', 'windowsButton', 'hotspotButton', 'sharedNetworkButton'];
  for (let i in radioButtons) {
    document.getElementById(radioButtons[i]).disabled = false;
  }
  // replace logo
  document.getElementById('qrcode').innerHTML = '<img src="assets/icon1024.png" style="width: 150px; height: 150px;">'
}

let disableUi = async () => {
  // hide start button
  startButton.style.display = 'none';
  // show cancel button
  cancelButton.style.display = '';
  // disable bluetooth switch
  document.getElementById('bluetoothSwitch').disabled = true;
  // disable send folder box
  document.getElementById('sendFolderCheckbox').disabled = true;
  // disable radio buttons, file/folder selection buttons
  let radioButtons = ['sendButton', 'receiveButton', 'androidButton', 'iosButton', 'linuxButton', 'macButton', 'windowsButton', 'hotspotButton', 'sharedNetworkButton'];
  for (let i in radioButtons) {
    document.getElementById(radioButtons[i]).disabled = true;
  }
}

window.startTransfer = startTransfer;
window.cancelTransfer = cancelTransfer;
window.selectFiles = selectFiles;
window.selectFolder = selectFolder;
window.bluetoothChange = bluetoothChange;
window.modeChange = modeChange;
window.peerChange = peerChange;
window.connectionModeChange = connectionModeChange;

const aboutMessage = `https://flyingcarpet.spiegl.dev
Version: 10.0.0
theron@spiegl.dev
Copyright (c) 2026, Theron Spiegl
All rights reserved.

Flying Carpet transfers files between two Android, iOS, Linux, macOS, and Windows devices over ad hoc WiFi. In Hotspot mode, no access point or shared network is required, just two WiFi cards in close range. Hotspot mode does not work from one Apple device (macOS or iOS) to another, because Apple no longer allows hotspots to be started programmatically: use Shared Network mode for those transfers.

In Shared Network mode, both devices must be connected to the same network. No hotspot is created: the devices find each other on the network automatically. Bluetooth is not used in this mode. The receiving device generates and displays a password, which must be entered on the sending device (or scanned on Android).

INSTRUCTIONS

Turn Bluetooth on or off on both devices. If one side fails to initialize Bluetooth or has it turned off, the other side must disable the "Use Bluetooth" switch in Flying Carpet.

Select Sending on one device and Receiving on the other. If not using Bluetooth, select the operating system of the other device. Click the "Start Transfer" button on each device. On the sending device, select the files or folder to send. On the receiving device, select the folder in which to receive files. (To send a folder, drag it onto the window instead of clicking "Start Transfer".)

If using Bluetooth, confirm the 6-digit PIN on each side. The WiFi connection will be configured automatically. If not using Bluetooth, you will need to scan a QR code or type in a password.

If prompted to join a WiFi network or modify WiFi settings, say Allow. On Windows you may have to grant permission to add a firewall rule. On macOS you may have to grant location permissions, which Apple requires to scan for WiFi networks. Flying Carpet does not read or collect your location, nor any other data.

TROUBLESHOOTING

If using Bluetooth fails, try manually unpairing the devices from one another and starting a new transfer.

If sending from macOS to Linux, you must first initiate pairing from the macOS System Settings > Bluetooth menu. Otherwise, disable Bluetooth on both sides and enter the password manually when prompted.

Flying Carpet may make multiple attempts to join the other device's hotspot.

Licensed under the GPL3: https://www.gnu.org/licenses/gpl-3.0.html#license-text`
