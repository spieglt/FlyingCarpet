// This file is required by the index.html file and will
// be executed in the renderer process for that window.
// All of the Node.js APIs are available in this process.
const path = require('path');

var modeGroup = document.getElementsByName('modeGroup');
var osGroup = document.getElementsByName('osGroup');
var sendForm = document.getElementById('sendForm');
var inputFile = document.getElementById('inputFile');
var fileButton = document.getElementById('fileButton');
var fileLabel = document.getElementById('fileLabel');
var receiveForm = document.getElementById('receiveForm');
var folderButton = document.getElementById('folderButton');
var folderChooser = document.getElementById('folderChooser');
var outputFile = document.getElementById('outputFile');
var startButton = document.getElementById('startButton');
var errorDiv = document.getElementById('errorDiv');

var peer = '';
var mode = '';

for(var i = 0; i < modeGroup.length; i++) {
   modeGroup[i].onclick = function() {
    var val = this.value;
    if(val == 'Sending'){
        sendForm.style.display = 'block';
        receiveForm.style.display = 'none';
        mode = 'Sending';
    }
    else if(val == 'Receiving'){
        sendForm.style.display = 'none';
        receiveForm.style.display = 'block';
        mode = 'Receiving';
    }
    errorDiv.innerHTML = '';
  }
}

for(var i = 0; i < osGroup.length; i++) {
   osGroup[i].onclick = function() {
    var val = this.value;
    if(val == 'macOS'){
      peer = 'macOS';
    }
    else if (val == 'Windows') {
      peer = 'Windows';
    }
  }
}

fileButton.onclick = () => {
    inputFile.click();
}

inputFile.onchange = () => {
    fileLabel.innerHTML = inputFile.files[0].path;
}

folderButton.onclick = () => {
    folderChooser.click();
}

folderChooser.onchange = function() {
    // outputFile.setAttribute('value', folderChooser.files[0].path + path.sep + 'outFile');
    outputFile.innerHTML = folderChooser.files[0].path + path.sep + 'outFile';
    console.log(folderChooser.files[0].path);
}

startButton.onclick = function () {

    if (mode == '') {
        errorDiv.innerHTML = 'Please select sending or receiving.';
        return;
    }
    if (peer == '') {
        errorDiv.innerHTML = 'Please select the OS of the other computer.';
        return;
    }

    obj = {};
    if (mode == 'Sending') {
        if (!inputFile.files[0]) {
            errorDiv.innerHTML = 'Please select a file to send.';
            return
        }
        obj.inFile = inputFile.files[0].path;
    } else if (mode == 'Receiving') {
        obj.outFile = outputFile.innerHTML;
        // check that file doesn't exist
    }
    obj.peer = peer;
    obj.mode = mode;

    console.log(obj);
}

