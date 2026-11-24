@echo off
rem Take r3cui off an Unfolded Circle Remote 3: switch back to the Normal UI, or remove r3cui completely.
rem
rem   uninstall.bat                        asks for everything
rem   uninstall.bat <IP> [PIN] [disable|remove]
rem
rem The warranty record written when r3cui was installed stays either way.
setlocal EnableExtensions

set "PAUSE="
if "%~1"=="" set "PAUSE=1"
set "IP=%~1"
set "PIN=%~2"
set "HOW=%~3"

where curl.exe >nul 2>nul || (echo This needs curl.exe, which comes with Windows 10 and newer. & goto done1)

if "%IP%"=="" set /p "IP=Remote's IP address: "
if "%PIN%"=="" for /f "usebackq delims=" %%p in (`powershell -NoProfile -Command "$s = Read-Host 'Web configurator PIN' -AsSecureString; [Runtime.InteropServices.Marshal]::PtrToStringAuto([Runtime.InteropServices.Marshal]::SecureStringToBSTR($s))"`) do set "PIN=%%p"

if "%HOW%"=="" (
  echo.
  echo   1^) Switch back to the Normal UI ^(r3cui stays installed, so it is easy to turn back on^)
  echo   2^) Remove r3cui completely
  echo.
  set "PICK="
  set /p "PICK=Choose 1 or 2: "
)
if "%HOW%"=="" if "%PICK%"=="1" set "HOW=disable"
if "%HOW%"=="" if "%PICK%"=="2" set "HOW=remove"
if not "%HOW%"=="disable" if not "%HOW%"=="remove" (echo Nothing was changed. & goto done1)

rem The PIN goes to curl in a temporary config file, so it never shows up in the process list.
set "CFG=%TEMP%\r3cui-%RANDOM%.cfg"
> "%CFG%" echo user = "web-configurator:%PIN%"

set "CODE=000"
if "%HOW%"=="disable" for /f %%c in ('curl.exe -sS -K "%CFG%" -o nul -w "%%{http_code}" --connect-timeout 8 -X PUT "http://%IP%/api/system/install/ui?enable=false" 2^>nul') do set "CODE=%%c"
if "%HOW%"=="remove" for /f %%c in ('curl.exe -sS -K "%CFG%" -o nul -w "%%{http_code}" --connect-timeout 8 -X DELETE "http://%IP%/api/system/install/ui" 2^>nul') do set "CODE=%%c"
del "%CFG%" >nul 2>nul

if "%CODE:~0,1%"=="2" (
  if "%HOW%"=="remove" (echo r3cui is removed. The remote is back on the Normal UI.) else (echo The remote is back on the Normal UI. r3cui is still installed.)
  goto done0
)
if "%CODE%"=="000" echo Can't reach a remote at %IP%. Check the address and that this computer is on the same network.
if "%CODE%"=="401" echo The remote rejected the PIN. Check the web configurator PIN in the remote's settings.
if "%CODE%"=="404" echo There is no custom UI installed on this remote.
if not "%CODE%"=="000" if not "%CODE%"=="401" if not "%CODE%"=="404" echo The remote answered with HTTP %CODE%; nothing may have changed.
goto done1

:done0
if defined PAUSE pause
exit /b 0

:done1
if defined PAUSE pause
exit /b 1
