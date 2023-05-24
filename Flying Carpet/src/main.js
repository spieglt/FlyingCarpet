const { dialog, os, path, tauri } = window.__TAURI__;
import { QRCode } from './deps/qrcode.js'

let aboutButton;
let selectionBox;
let outputBox;
let ssidBox;
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
    selectedMode: selectedMode,
    selectedPeer: selectedPeer,
    selectedFiles: selectedFiles,
    selectedFolder: selectedFolder,
    output: outputBox.innerText,
    transferRunning: startButton.style.display === 'none',
    passwordBoxValue: passwordBox.value,
    ssidBoxValue: ssidBox.value,
    progressBarValue: progressBar.value,
    progressBarVisible: progressBar.style.display !== 'none',
  };
  let uiJSON = JSON.stringify(uiState);
  sessionStorage.setItem('pageState', uiJSON);
}

window.addEventListener('DOMContentLoaded', async () => {
  aboutButton = document.getElementById('aboutButton');
  selectionBox = document.getElementById('selectionBox');
  outputBox = document.getElementById('outputBox');
  startButton = document.getElementById('startButton');
  cancelButton = document.getElementById('cancelButton');
  progressBar = document.getElementById('progressBar');

  appWindow = window.__TAURI__.window.appWindow;

  // about button
  aboutButton.onclick = () => {
    alert(aboutMessage);
  }

  // hide unnecessary buttons for macOS
  if (await os.type() === 'Darwin') {
    document.getElementById('iosSelector').style.display = 'none';
    document.getElementById('macSelector').style.display = 'none';
  }

  ssidBox = document.getElementById('ssidBox');
  ssidBox.onfocus = () => {
    if (ssidBox.value == '') {
      ssidBox.value = 'AndroidShare_';
    }
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

  // handle drag and drop
  await appWindow.listen('tauri://file-drop', async event => {
    if (selectedMode === 'send') {
      selectedFiles = await tauri.invoke('expand_files', { paths: event.payload });
    } else if (selectedMode === 'receive') {
      if (event.payload.length !== 1) {
        output('Error: if receiving, must drop only one destination folder.');
        return;
      }
      let is_dir = await tauri.invoke('is_dir', { path: event.payload[0] });
      if (is_dir) {
        selectedFolder = event.payload[0];
      } else {
        output('Error: if receiving, must select folder as destination.');
      }
    } else {
      output('Error: must select whether sending or receiving before dropping files or folder.');
    }
    checkStatus();
  })

  checkStatus();

  // rehydrate UI if user refreshed
  let uiState = JSON.parse(sessionStorage.getItem('pageState'));
  if (uiState) {
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
    ssidBox.value = uiState.ssidBoxValue;
    selectedFiles = uiState.selectedFiles;
    selectedFolder = uiState.selectedFolder;
    outputBox.innerText = uiState.output;
    progressBar.style.display = uiState.progressBarVisible ? '' : 'none';
    progressBar.value = uiState.progressBarValue;
    modeChange(selectedMode);
    updateSelectionBox();
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

async function startTransfer() {
  // handle password
  let [needPassword, needSsid] = await needPasswordAndSsid();
  let password;
  if (needPassword) {
    password = document.getElementById('passwordBox').value;
    if (password.length < 8) {
      output('Must enter password from the other device.');
      return;
    }
  } else {
    password = await tauri.invoke('generate_password');
    if (selectedPeer === 'ios' || selectedPeer === 'android') {
      output('\nStart the transfer on the other device and scan the QR code when prompted.');
      makeQRCode(password);
    } else {
      output(`Password: ${password}`);
      alert(`\nStart the transfer on the other device and enter this password when prompted:\n${password}`);
    }
  }

  // handle SSID if we're macOS connecting to Android
  let ssid = needSsid
    ? ssidBox.value
    : null;

  if (needSsid && ssid == '') {
    output('Must enter SSID. It will be displayed once you start the transfer on the Android device.');
    return;
  }

  // make sure we have a wifi interface and prompt for which if more than one
  let wifiInterface;
  let interfaces = await tauri.invoke('get_wifi_interfaces');
  console.log('interfaces:', interfaces);
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

  // disable UI
  disableUi();

  // kick off transfer
  await tauri.invoke('start_async', {
    mode: selectedMode,
    peer: selectedPeer,
    password: password,
    ssid: ssid,
    interface: wifiInterface,
    fileList: selectedFiles,
    receiveDir: selectedFolder,
    window: appWindow,
  });
}

async function cancelTransfer() {
  let startState = startButton.disabled;
  startButton.disabled = true;
  cancelButton.disabled = true;
  output(await tauri.invoke('cancel_transfer'));
  startButton.disabled = startState;
  cancelButton.disabled = false;
}

let selectFiles = async () => {
  let _selectedFiles = await dialog.open({
    multiple: true,
    directory: false,
  });
  if (_selectedFiles) { // don't let cancel clear selection
    selectedFiles = _selectedFiles;
  }
  checkStatus();
}

let selectFolder = async () => {
  let _selectedFolder = await dialog.open({
    multiple: false,
    directory: true,
  });
  if (_selectedFolder) { // don't let cancel clear selection
    selectedFolder = _selectedFolder;
  }
  checkStatus();
}

let updateSelectionBox = () => {
  let fileFolderBox = document.getElementById('fileFolderBox');
  let height = fileFolderBox.clientHeight;
  if (selectedFiles) {
    let s = '';
    for (let i in selectedFiles) {
      s += selectedFiles[i] + '\n';
    }
    selectionBox.innerText = 'Selected Files:\n' + s;
  } else if (selectedFolder) {
    selectionBox.innerText = 'Selected Folder:\n' + selectedFolder;
  } else {
    selectionBox.innerText = 'Drag and drop files/folders here or use button';
  }
  fileFolderBox.height = height + 'px';
}

let modeChange = async (button) => {
  // make proper button visible depending on mode. leave "Select Files" button visible if no mode selected on refresh.
  if (button === 'receive') {
    document.getElementById('filesButton').style.display = 'none';
    document.getElementById('folderButton').style.display = '';
  } else {
    document.getElementById('filesButton').style.display = '';
    document.getElementById('folderButton').style.display = 'none';
  }
  // only reset files/folder if mode was changed
  if (selectedMode != button) {
    selectedFiles = null;
    if (button === 'send') {
      selectedFolder = null;
    } else {
      selectedFolder = await path.desktopDir();
    }
  }
  selectedMode = button;
  checkStatus();
}

let peerChange = (button) => {
  selectedPeer = button;
  checkStatus();
}

let checkStatus = () => {
  updateSelectionBox();
  document.getElementById('filesButton').disabled = !selectedMode;
  document.getElementById('folderButton').disabled = !selectedMode;
  showPasswordAndSsid();
  startButton.disabled = !(selectedMode && selectedPeer
    && (selectedFiles || selectedFolder));
}

let needPasswordAndSsid = async () => {
  // if OS mac, always joining, always need password. also need ssid if peer == android.
  // if linux, joining windows, hosting mac/ios/android or linux if receiving.
  // if windows, always hosting unless windows and sending.
  let showPassword, showSsid;
  switch (await os.type()) {
    case 'Darwin':
      showPassword = true;
      showSsid = selectedPeer === 'android';
      break;
    case 'Linux':
      showPassword = selectedPeer === 'windows' || (selectedPeer === 'linux' && selectedMode === 'send');
      showSsid = false;
      break;
    case 'Windows_NT':
      showPassword = selectedPeer === 'windows' && selectedMode === 'send';
      showSsid = false;
      break;
    default:
      alert('Error in shouldShowPasswordAndSsid()');
  }
  return [showPassword, showSsid];
}

let showPasswordAndSsid = async () => {
  let [showPassword, showSsid] = await needPasswordAndSsid();
  if (showPassword) {
    document.getElementById('passwordBox').style.display = '';
  } else {
    document.getElementById('passwordBox').style.display = 'none';
  }
  if (showSsid) {
    ssidBox.style.display = '';
  } else {
    ssidBox.style.display = 'none';
  }
}

let enableUi = async () => {
  // show start button
  startButton.style.display = '';
  // hide cancel button
  cancelButton.style.display = 'none';
  // enable radio buttons, file/folder selection buttons
  let radioButtons = ['sendButton', 'receiveButton', 'androidButton', 'iosButton', 'linuxButton', 'macButton', 'windowsButton', 'filesButton', 'folderButton'];
  for (let i in radioButtons) {
    document.getElementById(radioButtons[i]).disabled = false;
  }
  // enable password and ssid boxes
  document.getElementById('passwordBox').disabled = false;
  document.getElementById('ssidBox').disabled = false;
  // replace logo
  document.getElementById('qrcode').innerHTML = '<img src="assets/icon1024.png" style="width: 150px; height: 150px;">'
}

let disableUi = async () => {
  // hide start button
  startButton.style.display = 'none';
  // show cancel button
  cancelButton.style.display = '';
  // disable radio buttons, file/folder selection buttons
  let radioButtons = ['sendButton', 'receiveButton', 'androidButton', 'iosButton', 'linuxButton', 'macButton', 'windowsButton', 'filesButton', 'folderButton'];
  for (let i in radioButtons) {
    document.getElementById(radioButtons[i]).disabled = true;
  }
  // disable password and ssid boxes
  document.getElementById('passwordBox').disabled = true;
  document.getElementById('ssidBox').disabled = true;
}

window.startTransfer = startTransfer;
window.cancelTransfer = cancelTransfer;
window.selectFiles = selectFiles;
window.selectFolder = selectFolder;
window.modeChange = modeChange;
window.peerChange = peerChange;

const aboutMessage = `https://flyingcarpet.spiegl.dev
Version: 7.1
theron@spiegl.dev
Copyright (c) 2023, Theron Spiegl
All rights reserved.

Flying Carpet performs file transfers between two laptops or phones (Android, iOS, Linux, Mac, Windows) via ad hoc WiFi. No access point or network gear is required. Just select a file, whether each device is sending or receiving, and the operating system of the other device. For mobile versions, search for "Flying Carpet File Transfer" in the Apple App Store or Google Play Store.

Redistribution and use in source and binary forms, with or without modification, are permitted provided that the following conditions are met:

* Redistributions of source code must retain the above copyright notice, this list of conditions and the following disclaimer.

* Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the following disclaimer in the documentation and/or other materials provided with the distribution.

* Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote products derived from this software without specific prior written permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.`
