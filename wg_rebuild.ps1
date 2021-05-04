# clean up old executable
# rm "./Flying Carpet.exe"

# copy file with icon for real and wrapper programs
# fc.syso is automatically detected for inclusion
# by `go build` and `qtdeploy` during compilation
Copy-Item .\icons\Windows\fc.syso .\gui\flyingcarpet\
Copy-Item .\icons\Windows\fc.syso .\gui\

# copy WiFi Direct DLL to core directory for CLI
Copy-Item .\WFD_DLL\x64\Release\WFD_DLL.dll .\core\wfd.dll

# build with github.com/therecipe/qt
qtdeploy.exe build desktop .\gui\flyingcarpet

# use Windows SDK utility to embed application manifest,
# which will force Flying Carpet to launch "as Administrator"
# if the user has UAC enabled
mt.exe -manifest .\gui\flyingcarpet\flyingcarpet.exe.manifest -outputresource:.\gui\flyingcarpet\deploy\windows\flyingcarpet.exe;1

# copy WiFi Direct DLL to Qt output directory
Copy-Item .\WFD_DLL\x64\Release\WFD_DLL.dll .\gui\flyingcarpet\deploy\windows\wfd.dll

# zip to /bin
New-Item -ItemType Directory -Force -Path .\bin
Compress-Archive -Force -Path '.\gui\flyingcarpet\deploy\windows\*' -DestinationPath '.\bin\Flying Carpet (Windows).zip'

# execute
cmd /C .\gui\flyingcarpet\deploy\windows\flyingcarpet.exe
