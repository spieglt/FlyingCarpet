# clean up old executable
rm "./Flying Carpet.exe"

# copy file with icon
Copy-Item .\icons\Windows\fc.syso .\gui\flyingcarpet\
Copy-Item .\icons\Windows\fc.syso .\gui\

# build with github.com/therecipe/qt
cd .\gui\flyingcarpet
qtdeploy.exe build desktop
cd ..\..

# use Windows SDK utility to embed application manifest, 
# which will force Flying Carpet to launch "as Administrator" if the user has UAC enabled
mt.exe -manifest .\gui\flyingcarpet\flyingcarpet.exe.manifest -outputresource:.\gui\flyingcarpet\deploy\windows\flyingcarpet.exe;1

# copy WiFi Direct DLL to Qt output directory
Copy-Item .\WFD_DLL\x64\Release\WFD_DLL.dll .\gui\flyingcarpet\deploy\windows\wfd.dll

# bundle
cd gui
rice.exe embed-go
# extra flags prevent console window from showing while wrapper extracts files to temp directory
go build -ldflags -H=windowsgui -o "Flying Carpet.exe"

# execute
mv -Force "Flying Carpet.exe" ..
cd ..
cmd /C "./Flying Carpet.exe"