!include LogicLib.nsh
!include FileFunc.nsh

!define BINARY_DIR "$INSTDIR\resources-windows\binaries"
!define WIREGUARD_ZIP_URL "https://download.wireguard.com/wireguard-nt/wireguard-nt-1.1.zip"
!define WIREGUARD_ZIP_NAME "wireguard-nt-1.1.zip"

!macro NSIS_HOOK_PREINSTALL
  SetOutPath "$TEMP"
  CreateDirectory "${BINARY_DIR}"

  ; If the app already has a runtime DLL in its resources folder, keep it.
  ${IfNot} ${FileExists} "${BINARY_DIR}\wireguard-amd64.dll"
  ${AndIfNot} ${FileExists} "${BINARY_DIR}\wireguard-arm64.dll"
    ; Prefer a pre-existing WireGuard install on the machine.
    ${If} ${FileExists} "$PROGRAMFILES64\WireGuard\wireguard-amd64.dll"
      CopyFiles "$PROGRAMFILES64\WireGuard\wireguard-amd64.dll" "${BINARY_DIR}\wireguard-amd64.dll"
    ${ElseIf} ${FileExists} "$PROGRAMFILES\WireGuard\wireguard-amd64.dll"
      CopyFiles "$PROGRAMFILES\WireGuard\wireguard-amd64.dll" "${BINARY_DIR}\wireguard-amd64.dll"
    ${ElseIf} ${FileExists} "$PROGRAMFILES64\WireGuard\wireguard-arm64.dll"
      CopyFiles "$PROGRAMFILES64\WireGuard\wireguard-arm64.dll" "${BINARY_DIR}\wireguard-arm64.dll"
    ${ElseIf} ${FileExists} "$PROGRAMFILES\WireGuard\wireguard-arm64.dll"
      CopyFiles "$PROGRAMFILES\WireGuard\wireguard-arm64.dll" "${BINARY_DIR}\wireguard-arm64.dll"
    ${Else}
      ; Fall back to downloading the official WireGuard NT release zip.
      nsExec::ExecToLog '"$WINDIR\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -Command "$ErrorActionPreference = \"Stop\"; $zip = \"$TEMP\${WIREGUARD_ZIP_NAME}\"; $dest = \"$TEMP\wireguard-nt\"; Invoke-WebRequest -Uri \"${WIREGUARD_ZIP_URL}\" -OutFile $zip; Expand-Archive -Path $zip -DestinationPath $dest -Force; $dll = if (Test-Path \"$dest\bin\amd64\wireguard.dll\") { \"$dest\bin\amd64\wireguard.dll\" } elseif (Test-Path \"$dest\bin\arm64\wireguard.dll\") { \"$dest\bin\arm64\wireguard.dll\" } elseif (Test-Path \"$dest\bin\x86\wireguard.dll\") { \"$dest\bin\x86\wireguard.dll\" } else { $null }; if ($dll) { Copy-Item $dll -Destination \"${BINARY_DIR}\wireguard-amd64.dll\" -Force }; $wg = if (Test-Path \"$dest\bin\amd64\wg.exe\") { \"$dest\bin\amd64\wg.exe\" } elseif (Test-Path \"$dest\bin\arm64\wg.exe\") { \"$dest\bin\arm64\wg.exe\" } elseif (Test-Path \"$dest\bin\x86\wg.exe\") { \"$dest\bin\x86\wg.exe\" } else { $null }; if ($wg) { Copy-Item $wg -Destination \"${BINARY_DIR}\wg.exe\" -Force }"'
    ${EndIf}
  ${EndIf}
!macroend
