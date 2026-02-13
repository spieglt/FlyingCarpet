const { core, dialog, os } = window.__TAURI__;
import { QRCode } from './deps/qrcode.js'
import { initI18n, t, getLocale, setLocale, applyTranslations } from './i18n.js'

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
let languageSelector;

let selectedMode;
let selectedPeer;
let selectedFiles;
let selectedFolder;

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
    passwordBoxValue: passwordBox.value,
    progressBarValue: progressBar.value,
    progressBarVisible: progressBar.style.display !== 'none',
    locale: getLocale(),
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
  languageSelector = document.getElementById('languageSelector');

  appWindow = window.__TAURI__.window.getCurrentWindow();

  // initialize i18n
  await initI18n();
  languageSelector.value = getLocale();
  applyTranslations();
  output(t('welcome_message'));

  // language selector
  languageSelector.onchange = async () => {
    await setLocale(languageSelector.value);
    // update dynamic text
    if (selectedMode) {
      modeChange(selectedMode);
    }
  };

  // check for bluetooth support
  let error = await core.invoke('check_support');
  if (error != null) {
    output(t('bluetooth_init_failed', { error }));
    bluetoothSwitch.disabled = true;
    bluetoothSwitch.checked = false;
    usingBluetooth = false;
    canUseBluetooth = false;
  } else {
    output(t('bluetooth_supported'));
    bluetoothSwitch.disabled = false;
    bluetoothSwitch.checked = true;
    usingBluetooth = true;
    canUseBluetooth = true;
  }

  // about button
  aboutButton.onclick = () => {
    alert(t('about_message'));
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
    let choice = await dialog.ask(t('confirm_bluetooth_pin', { pin: event.payload.message }), { title: t('confirm_bluetooth_title'), type: 'info' });
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
        output(t('error_receiving_drop_one_folder'));
        return;
      }
      let is_dir = await core.invoke('is_dir', { path: event.payload[0] });
      if (is_dir) {
        selectedFolder = event.payload[0];
      } else {
        output(t('error_receiving_must_select_folder'));
      }
      startTransfer(true);
    } else {
      output(t('error_must_select_mode_before_drop'));
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
    passwordBox.value = uiState.passwordBoxValue;
    selectedFiles = uiState.selectedFiles;
    selectedFolder = uiState.selectedFolder;
    outputBox.innerText = uiState.output;
    progressBar.style.display = uiState.progressBarVisible ? '' : 'none';
    progressBar.value = uiState.progressBarValue;
    if (uiState.locale) {
      await setLocale(uiState.locale);
      languageSelector.value = uiState.locale;
    }
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
      output(t('must_enter_password'));
      return;
    }
  }

  // make sure we have a wifi interface and prompt for which if more than one
  let wifiInterface;
  let interfaces = await core.invoke('get_wifi_interfaces');
  // console.log('interfaces:', interfaces);
  switch (interfaces.length) {
    case 0:
      output(t('no_wifi_interfaces'));
      return;
    case 1:
      wifiInterface = interfaces[0];
      break;
    default:
      let alertString = t('choose_interface_prompt');
      for (let i = 0; i < interfaces.length; i++) {
        alertString += `${i+1}: ${interfaces[i][0]}\n`
      }
      let choice = parseInt(prompt(alertString));
      if (choice && choice > 0 && choice <= interfaces.length) {
        wifiInterface = interfaces[choice - 1];
        output(t('using_interface', { interface: wifiInterface[0] }));
      } else {
        output(t('invalid_interface'));
        return;
      }
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
          output(t('user_cancelled'));
          return;
        }
        selectedFiles = await core.invoke('expand_files', { paths: [folder] });
      } else {
        await selectFiles();
        if (!selectedFiles) {
          output(t('user_cancelled'));
          return;
        }
      }
    } else if (selectedMode == 'receive') {
      await selectFolder();
      if (!selectedFolder) {
        output(t('user_cancelled'));
        return;
      }
    } else {
      output(t('must_select_mode'));
      return;
    }
  }

  // if we're hosting, generate and display the password
  if (!await needPassword()) {
    if (!usingBluetooth) {
      password = await core.invoke('generate_password');
      if (selectedPeer === 'ios' || selectedPeer === 'android') {
        output(t('start_transfer_scan_qr'));
        makeQRCode(password);
      } else {
        output(t('password_display', { password }));
        alert(t('start_transfer_enter_password', { password }));
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
  startButton.innerText = button === 'receive' ? t('select_folder') : t('select_files');
  document.getElementById('sendFolderDiv').style.display = button === 'send' ? '' : 'none';
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
      alert(t('error_in_needPassword'));
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
  // enable send folder box
  document.getElementById('sendFolderCheckbox').disabled = false;
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
  // disable send folder box
  document.getElementById('sendFolderCheckbox').disabled = true;
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
