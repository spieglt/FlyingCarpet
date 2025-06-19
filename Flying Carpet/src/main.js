const { core, dialog, os } = window.__TAURI__;
import { QRCode } from './deps/qrcode.js'

let aboutButton;
let canUseBluetooth = false;
let usingBluetooth;
let bluetoothSwitch;
let peerLabel;
let peerBox;
let outputBox;
let startButton;
let cancelButton;
let progressBar;
let appWindow;

let selectedMode;
let selectedPeer;
let selectedFiles;
let selectedFolder;

// save UI if user refreshes
window.onunload = () => {
  let uiState = {
    usingBluetooth: usingBluetooth,
    // canUseBluetooth:
    selectedMode: selectedMode,
    selectedPeer: selectedPeer,
    selectedFiles: selectedFiles,
    selectedFolder: selectedFolder,
    output: outputBox.innerText,
    transferRunning: startButton.style.display === 'none',
    passwordBoxValue: passwordBox.value,
    progressBarValue: progressBar.value,
    progressBarVisible: progressBar.style.display !== 'none',
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
    alert(aboutMessage);
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
    passwordBox.value = uiState.passwordBoxValue;
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

  // if we need password, make sure we have it before prompting for files/folder
  let password = null;
  if (await needPassword()) {
    password = document.getElementById('passwordBox').value;
    if (password.length < 8) {
      output('Must enter password from the other device.');
      return;
    }
  }

  // make sure we have a wifi interface and prompt for which if more than one
  let wifiInterface;
  let interfaces = await core.invoke('get_wifi_interfaces');
  // console.log('interfaces:', interfaces);
  switch (interfaces.length) {
    case 0:
      output('No WiFi interfaces found. Flying Carpet only works over WiFi.');
      return;
    case 1:
      wifiInterface = interfaces[0];
      break;
    default:
      let alertString = 'Enter the number for which WiFi interface to use (e.g. "1" or "2"):\n'
      for (let i = 0; i < interfaces.length; i++) {
        alertString += `${i+1}: ${interfaces[i][0]}\n`
      }
      let choice = parseInt(prompt(alertString));
      if (choice && choice > 0 && choice <= interfaces.length) {
        wifiInterface = interfaces[choice - 1];
        output(`Using interface: ${wifiInterface[0]}`);
      } else {
        output('Invalid interface selected. Please enter just the number of the WiFi interface you would like to use, e.g. "1" or "3".');
        return;
      }
  }
  
  // get files or folder
  if (!filesSelected) {
    if (selectedMode == 'send') {
      await selectFiles();
      if (!selectedFiles) {
        output('User cancelled.');
        return;
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
  
  // if we're hosting, generate and display the password
  if (!await needPassword()) {
    if (!usingBluetooth) {
      password = await core.invoke('generate_password');
      if (selectedPeer === 'ios' || selectedPeer === 'android') {
        output('\nStart the transfer on the other device and scan the QR code when prompted.');
        makeQRCode(password);
      } else {
        output(`Password: ${password}`);
        alert(`\nStart the transfer on the other device and enter this password when prompted:\n${password}`);
      }
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
  selectedMode = button;
  checkStatus();
}

let peerChange = (button) => {
  selectedPeer = button;
  checkStatus();
}

let checkStatus = () => {
  showPassword();
  if (usingBluetooth) {
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

let showPassword = async () => {
  let showPassword = await needPassword();
  if (showPassword) {
    document.getElementById('passwordBox').style.display = '';
  } else {
    document.getElementById('passwordBox').style.display = 'none';
  }
}

let enableUi = async () => {
  // show start button
  startButton.style.display = '';
  // hide cancel button
  cancelButton.style.display = 'none';
  // enable bluetooth switch
  if (canUseBluetooth) {
    document.getElementById('bluetoothSwitch').disabled = false;
  }
  // enable radio buttons, file/folder selection buttons
  let radioButtons = ['sendButton', 'receiveButton', 'androidButton', 'iosButton', 'linuxButton', 'macButton', 'windowsButton'];
  for (let i in radioButtons) {
    document.getElementById(radioButtons[i]).disabled = false;
  }
  // enable password box
  document.getElementById('passwordBox').disabled = false;
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
  // disable radio buttons, file/folder selection buttons
  let radioButtons = ['sendButton', 'receiveButton', 'androidButton', 'iosButton', 'linuxButton', 'macButton', 'windowsButton'];
  for (let i in radioButtons) {
    document.getElementById(radioButtons[i]).disabled = true;
  }
  // disable password box
  document.getElementById('passwordBox').disabled = true;
}

window.startTransfer = startTransfer;
window.cancelTransfer = cancelTransfer;
window.selectFiles = selectFiles;
window.selectFolder = selectFolder;
window.bluetoothChange = bluetoothChange;
window.modeChange = modeChange;
window.peerChange = peerChange;

const aboutMessage = `https://flyingcarpet.spiegl.dev
Version: 9.0.0
theron@spiegl.dev
Copyright (c) 2025, Theron Spiegl
All rights reserved.

Flying Carpet transfers files between two Android, iOS, Linux, macOS, and Windows devices over ad hoc WiFi. No access point or shared network is required, just two WiFi cards in close range. The only non-working pairings are from one Apple device (macOS or iOS) to another, because Apple no longer allows hotspots to be started programmatically.

INSTRUCTIONS

Turn Bluetooth on or off on both devices. If one side fails to initialize Bluetooth or has it turned off, the other side must disable the "Use Bluetooth" switch in Flying Carpet.

Select Sending on one device and Receiving on the other. If not using Bluetooth, select the operating system of the other device. Click the "Start Transfer" button on each device. On the sending device, select the files or folder to send. On the receiving device, select the folder in which to receive files. (To send a folder, drag it onto the window instead of clicking "Start Transfer".)

If using Bluetooth, confirm the 6-digit PIN on each side. The WiFi connection will be configured automatically. If not using Bluetooth, you will need to scan a QR code or type in a password.

If prompted to join a WiFi network or modify WiFi settings, say Allow. On Windows you may have to grant permission to add a firewall rule. On macOS you may have to grant location permissions, which Apple requires to scan for WiFi networks. Flying Carpet does not read or collect your location, nor any other data.

TROUBLESHOOTING

Android and iOS devices must be kept awake with Flying Carpet in the foreground for the duration of the transfer, or the WiFi connection may drop.

If using Bluetooth fails, try manually unpairing the devices from one another and starting a new transfer.

If sending from macOS to Linux, you must first initiate pairing from the macOS System Settings > Bluetooth menu. Otherwise, disable Bluetooth on both sides and enter the password manually when prompted.

Flying Carpet may make multiple attempts to join the other device's hotspot.

Licensed under the GPL3: https://www.gnu.org/licenses/gpl-3.0.html#license-text`
