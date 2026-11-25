$ErrorActionPreference = 'Stop'
$setup = $env:PERFUSE_UPDATE_SETUP
$client = $env:PERFUSE_UPDATE_APP
$previous = $env:PERFUSE_UPDATE_PREVIOUS
$updateLock = $null
$closed = $false
try {
    $old = Get-Process -Id ([int]$env:PERFUSE_UPDATE_PID) -ErrorAction SilentlyContinue
    if ($old -and !$old.WaitForExit(60000)) {
        throw 'Perfuse did not close. Please close it and try the update again.'
    }
    $closed = $true
    $lockPath = Join-Path $env:LOCALAPPDATA 'Perfuse/.client-update'
    [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($lockPath)) | Out-Null
    $updateLock = [IO.File]::Open($lockPath, 'OpenOrCreate', 'Write', 'None')
    $result = Start-Process -FilePath $setup -ArgumentList '/VERYSILENT', '/NORESTART' -WindowStyle Hidden -PassThru -Wait
    if ($result.ExitCode -ne 0) {
        throw ('The Perfuse installer exited with code {0}.' -f $result.ExitCode)
    }
    Start-Process -FilePath $client
}
catch {
    Add-Type -AssemblyName System.Windows.Forms
    [Windows.Forms.MessageBox]::Show($_.Exception.Message, 'Perfuse update') | Out-Null
    if ($closed -and $updateLock -and (Test-Path -LiteralPath $previous)) {
        Start-Process -FilePath $previous
    }
}
finally {
    if ($updateLock) { $updateLock.Dispose() }
    Remove-Item -LiteralPath $setup -Force -ErrorAction SilentlyContinue
    if (!(Test-Path -LiteralPath $setup)) {
        [IO.Directory]::Delete([IO.Path]::GetDirectoryName($setup))
    }
}
