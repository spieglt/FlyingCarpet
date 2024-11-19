const { core, dialog, os } = window.__TAURI__;
import { QRCode } from './deps/qrcode.js'

let aboutButton;
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
  } else {
    output('Bluetooth is supported.');
    bluetoothSwitch.disabled = false;
    bluetoothSwitch.checked = true;
    usingBluetooth = true;
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

  // handle drag and drop
  await appWindow.listen('tauri://file-drop', async event => {
    if (selectedMode === 'send') {
      selectedFiles = await core.invoke('expand_files', { paths: event.payload });
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
  })

  checkStatus();

  // rehydrate UI if user refreshed
  let uiState = JSON.parse(sessionStorage.getItem('pageState'));
  if (uiState) {
    bluetoothSwitch.checked = uiState.usingBluetooth;
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
  switch (await os.type()) {
    case 'Linux':
      showPassword = selectedPeer === 'windows' || (selectedPeer === 'linux' && selectedMode === 'send');
      break;
    case 'Windows_NT':
      showPassword = selectedPeer === 'windows' && selectedMode === 'send';
      break;
    default:
      alert('Error in needPassword()'); // TODO: this happening on linux
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
  document.getElementById('bluetoothSwitch').disabled = false; // TODO: only enable if we can use bluetooth. need canUseBluetooth?
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
Version: 8.0.1
theron@spiegl.dev
Copyright (c) 2024, Theron Spiegl
All rights reserved.

Flying Carpet performs file transfers between two laptops or phones (Android, iOS, Linux, macOS, Windows) via ad hoc WiFi. No access point or network gear is required. Just select a file, whether each device is sending or receiving, and the operating system of the other device. For mobile versions, search for "Flying Carpet File Transfer" in the Apple App Store or Google Play Store.

Licensed under the GPL3: https://www.gnu.org/licenses/gpl-3.0.html#license-text`
