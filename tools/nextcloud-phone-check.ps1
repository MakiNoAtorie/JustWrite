# tool: run the phone path against a real Nextcloud; asks for the app password | run: .\tools\nextcloud-phone-check.ps1 -Server https://host -User name -Project Novel.jw
# With -Project: opens that project and reads it; nothing changes. With no -Project: a small test
# project goes up, is written from the phone side, checked, and removed from the server.
param(
    [Parameter(Mandatory)][string]$Server,
    [Parameter(Mandatory)][string]$User,
    [string]$Project = ""
)
$env:JUSTWRITE_TEST_NEXTCLOUD = $Server
$env:JUSTWRITE_TEST_USER = $User
$env:JUSTWRITE_TEST_PROJECT = if ($Project) { $Project } else { $null }
$secure = Read-Host "App password" -AsSecureString
$env:JUSTWRITE_TEST_PASSWORD = [System.Net.NetworkCredential]::new("", $secure).Password
cargo test --lib a_real_nextcloud_takes_the_phone -- --ignored --nocapture
$env:JUSTWRITE_TEST_PASSWORD = $null
