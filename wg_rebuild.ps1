# clean up old executable
rm "./Flying Carpet.exe"

# copy file with icon for real and wrapper programs
# fc.syso is automatically detected for inclusion
# by `go build` and `qtdeploy` during compilation
Copy-Item .\icons\Windows\fc.syso .\gui\flyingcarpet\
Copy-Item .\icons\Windows\fc.syso .\gui\

# build with github.com/therecipe/qt
qtdeploy.exe build desktop .\gui\flyingcarpet

# use Windows SDK utility to embed application manifest,
# which will force Flying Carpet to launch "as Administrator"
# if the user has UAC enabled
mt.exe -manifest .\gui\flyingcarpet\flyingcarpet.exe.manifest -outputresource:.\gui\flyingcarpet\deploy\windows\flyingcarpet.exe;1

# copy WiFi Direct DLL to Qt output directory
Copy-Item .\WFD_DLL\x64\Release\WFD_DLL.dll .\gui\flyingcarpet\deploy\windows\wfd.dll

# bundle
cd gui
rice.exe embed-go
# extra flags prevent console window from showing while wrapper extracts files to temp directory
go build -ldflags -H=windowsgui -o "Flying Carpet.exe"
mv -Force "Flying Carpet.exe" ..
cd ..

# zip to /bin
Compress-Archive -Force -Path '.\Flying Carpet.exe' -DestinationPath '.\bin\Flying Carpet (Windows).zip'

# execute
cmd /C "./Flying Carpet.exe"