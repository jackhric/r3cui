@echo off
rem Install r3cui on an Unfolded Circle Remote 3, from a Windows computer on the same network.
rem
rem   install.bat                          asks for everything
rem   install.bat <IP> [PIN] [file.tar.gz] uses a local archive instead of the latest release
rem
rem Installing a custom UI voids the remote's warranty (the remote keeps a permanent record of it).
setlocal EnableExtensions

set "REPO=jackhric/r3cui"
set "URL=https://github.com/%REPO%/releases/latest/download/r3cui.tar.gz"
set "PAUSE="
if "%~1"=="" set "PAUSE=1"
set "IP=%~1"
set "PIN=%~2"
set "FILE=%~3"

where curl.exe >nul 2>nul || (echo This needs curl.exe, which comes with Windows 10 and newer. & goto done1)

if "%IP%"=="" set /p "IP=Remote's IP address: "
if "%PIN%"=="" for /f "usebackq delims=" %%p in (`powershell -NoProfile -Command "$s = Read-Host 'Web configurator PIN' -AsSecureString; [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($s))"`) do set "PIN=%%p"

rem The PIN goes to curl in a temporary config file, so it never shows up in the process list.
set "CFG=%TEMP%\r3cui-%RANDOM%.cfg"
> "%CFG%" echo user = "web-configurator:%PIN%"

echo Checking the remote at %IP%...
set "CODE=000"
for /f %%c in ('curl.exe -sS -K "%CFG%" -o nul -w "%%{http_code}" --connect-timeout 8 "http://%IP%/api/system/install/ui" 2^>nul') do set "CODE=%%c"
if "%CODE%"=="000" (echo Can't reach a remote at %IP%. Check the address and that this computer is on the same network. & goto fail)
if "%CODE%"=="401" (echo The remote rejected the PIN. Check the web configurator PIN in the remote's settings. & goto fail)

echo.
echo   Installing r3cui voids your Remote 3 warranty. The remote keeps a
echo   permanent record that a custom UI was installed. That record stays
echo   even if you go back to the Normal UI later.
echo.
set "ANSWER="
set /p "ANSWER=Type yes to install r3cui: "
if /i not "%ANSWER%"=="yes" (echo Nothing was installed. & goto fail)

if "%FILE%"=="" (
  set "FILE=%TEMP%\r3cui.tar.gz"
  echo Downloading the latest r3cui release...
  curl.exe -fL --progress-bar -o "%TEMP%\r3cui.tar.gz" "%URL%" || (echo The download failed. Try again, or pass a r3cui.tar.gz you downloaded. & goto fail)
)
if not exist "%FILE%" (echo No file at %FILE%. & goto fail)

echo Installing. This can take a minute...
set "CODE=000"
for /f %%c in ('curl.exe -sS -K "%CFG%" -o nul -w "%%{http_code}" "http://%IP%/api/system/install/ui?void_warranty=yes" --form "file=@%FILE%" 2^>nul') do set "CODE=%%c"
if "%CODE:~0,1%"=="2" (
  echo Installed. The remote switches to r3cui on its own; if it doesn't, restart the remote.
  echo To go back to the Normal UI, run uninstall.bat.
  del "%CFG%" >nul 2>nul
  goto done0
)
if "%CODE%"=="400" echo The remote says the file is not a valid r3cui package.
if "%CODE%"=="401" echo The remote rejected the PIN.
if "%CODE%"=="403" echo The remote refused the install (it needs the warranty acknowledgement and an admin PIN).
if "%CODE%"=="409" echo This r3cui needs newer remote firmware. Update the remote and try again.
if "%CODE%"=="507" echo The remote is out of storage.
if "%CODE%"=="000" echo Lost the connection to the remote during the install.
echo The remote answered with HTTP %CODE%.

:fail
del "%CFG%" >nul 2>nul
goto done1

:done0
if defined PAUSE pause
exit /b 0

:done1
if defined PAUSE pause
exit /b 1
