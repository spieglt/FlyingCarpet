mkdir -Force .\static
Copy-Item -Force ..\WFD_DLL\x64\Release\WFD_DLL.dll .\static\wfd.dll
rice.exe embed-go
