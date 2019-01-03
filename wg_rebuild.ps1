
Copy-Item .\WFD_DLL\x64\Release\WFD_DLL.dll .\static\wfd.dll
mkdir static
go-bindata -o static.go static\

Copy-Item .\icons\Windows\fc.syso .\gui\flyingcarpet\

qtdeploy.exe build desktop .\gui\flyingcarpet

mt.exe -manifest .\gui\flyingcarpet\flyingcarpet.exe.manifest -outputresource:.\gui\flyingcarpet\deploy\windows\flyingcarpet.exe;1

# bundle

.\gui\flyingcarpet\deploy\windows\flyingcarpet.exe
