# Exercise release decisions and the external Authenticode verification boundary without certificates.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'release-signing.ps1')
$passed = 0

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw $Message }
    $script:passed++
}

function Assert-Rejected([scriptblock]$Operation, [string]$Expected) {
    $message = ''
    try { & $Operation | Out-Null } catch { $message = $_.Exception.Message }
    Assert-True ($message.Contains($Expected)) "Expected rejection containing '$Expected'; got '$message'"
}

$names = @('SIGNPATH_ORGANIZATION_ID', 'SIGNPATH_PROJECT_SLUG',
    'SIGNPATH_SIGNING_POLICY_SLUG', 'SIGNPATH_API_TOKEN')
for ($mask = 0; $mask -lt 16; $mask++) {
    $configuration = @{}
    for ($index = 0; $index -lt $names.Count; $index++) {
        $configuration[$names[$index]] = if ($mask -band (1 -shl $index)) { 'fixture-value' } else { ' ' }
    }
    if ($mask -eq 15) {
        $decision = Get-ReleaseSigningDecision 'tag' 'push' $configuration
        Assert-True ($decision.Required -and $decision.Enabled) 'A fully configured tag must sign.'
    }
    else {
        Assert-Rejected { Get-ReleaseSigningDecision 'tag' 'push' $configuration } 'requires code signing'
    }
    foreach ($eventName in @('pull_request', 'workflow_dispatch')) {
        $decision = Get-ReleaseSigningDecision 'branch' $eventName $configuration
        Assert-True (-not $decision.Enabled -and -not $decision.Required) 'A branch build must not sign.'
    }
}

$configured = @{}
foreach ($name in $names) { $configured[$name] = 'fixture-value' }
$decision = Get-ReleaseSigningDecision 'tag' 'workflow_dispatch' $configured
Assert-True ($decision.Enabled) 'A manual tag build must still require signing.'
$decision = Get-ReleaseSigningDecision 'tag' 'pull_request' $configured
Assert-True (-not $decision.Enabled) 'PR builds must never use signing credentials.'

$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('ubot-release-signing-' + [Guid]::NewGuid().ToString('N'))
$null = New-Item -ItemType Directory -Path $testRoot
$application = Join-Path $testRoot 'application.exe'
$installer = Join-Path $testRoot 'installer.exe'
[IO.File]::WriteAllText($application, 'Unsigned application fixture')
[IO.File]::WriteAllText($installer, 'Unsigned installer fixture')
$cliEnvironmentFiles = [System.Collections.Generic.List[string]]::new()

function Invoke-SigningCli {
    param([string]$RefType, [string]$EventName, [hashtable]$Configuration,
        [string]$Action = 'CheckConfiguration', [string]$Artifact)

    # Isolate every environment override in the child; never expose inherited signing credentials.
    $start = [System.Diagnostics.ProcessStartInfo]::new()
    $start.FileName = Join-Path $PSHOME 'pwsh.exe'
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    foreach ($name in $names) {
        $null = $start.Environment.Remove($name)
        if ($Configuration.ContainsKey($name)) { $start.Environment[$name] = $Configuration[$name] }
    }
    $null = $start.Environment.Remove('SIGN')
    $environmentFile = Join-Path $testRoot ([Guid]::NewGuid().ToString('N') + '.env')
    $cliEnvironmentFiles.Add($environmentFile)
    $start.Environment['GITHUB_ENV'] = $environmentFile
    $start.Environment['GITHUB_REF_TYPE'] = $RefType
    $start.Environment['GITHUB_EVENT_NAME'] = $EventName
    $start.Environment['UBOT_TEST_SIGNING_SCRIPT'] = Join-Path $PSScriptRoot 'release-signing.ps1'
    $start.Environment['UBOT_TEST_ACTION'] = $Action
    $start.Environment['UBOT_TEST_ARTIFACT'] = $Artifact
    # Invoke the script, rather than dot-source it, so its CLI dispatch runs. Capture the precise
    # exception message without PowerShell's terminal wrapping or source-line formatting.
    $command = @'
try {
    if ($env:UBOT_TEST_ACTION -eq 'VerifyArtifacts') {
        & $env:UBOT_TEST_SIGNING_SCRIPT -Action VerifyArtifacts -Path $env:UBOT_TEST_ARTIFACT
    } else {
        & $env:UBOT_TEST_SIGNING_SCRIPT
    }
} catch {
    [Console]::Error.WriteLine($_.Exception.Message)
    exit 1
}
'@
    foreach ($argument in @('-NoProfile', '-NonInteractive', '-EncodedCommand',
        [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($command)))) {
        $start.ArgumentList.Add($argument)
    }
    $process = [System.Diagnostics.Process]::new()
    $process.StartInfo = $start
    try {
        $null = $process.Start()
        $stdout = $process.StandardOutput.ReadToEndAsync()
        $stderr = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(30000)) {
            $process.Kill($true)
            throw 'Signing CLI regression subprocess timed out.'
        }
        [pscustomobject]@{ ExitCode = $process.ExitCode; Output = $stdout.GetAwaiter().GetResult().Trim()
            Error = $stderr.GetAwaiter().GetResult().Trim(); EnvironmentFile = $environmentFile }
    }
    finally { $process.Dispose() }
}
try {
    $valid = [pscustomobject]@{ Status = 'Valid'; SignerCertificate = [pscustomobject]@{ Subject = 'Fixture signer' } }
    $signatures = @{ 'application.exe' = $valid; 'installer.exe' = $valid }
    $reader = { param($file) $signatures[[IO.Path]::GetFileName($file)] }
    Assert-ReleaseArtifactSignatures -LiteralPath @($application, $installer) -SignatureReader $reader
    Assert-True $true 'Both signed artifacts are accepted.'
    foreach ($artifact in @('application.exe', 'installer.exe')) {
        foreach ($status in @('NotSigned', 'HashMismatch', 'NotTrusted', 'UnknownError')) {
            $signatures[$artifact] = [pscustomobject]@{ Status = $status; SignerCertificate = $valid.SignerCertificate }
            Assert-Rejected { Assert-ReleaseArtifactSignatures @($application, $installer) $reader } 'valid Authenticode'
        }
        $signatures[$artifact] = [pscustomobject]@{ Status = 'Valid'; SignerCertificate = $null }
        Assert-Rejected { Assert-ReleaseArtifactSignatures @($application, $installer) $reader } 'valid Authenticode'
        $signatures[$artifact] = $valid
    }
    Assert-Rejected { Assert-ReleaseArtifactSignatures @($application, (Join-Path $testRoot 'missing.exe')) $reader } 'missing'

    $result = Invoke-SigningCli 'tag' 'push' @{}
    $expected = 'A tagged release requires code signing. Configure: ' + ($names -join ', ') +
        '. No release artifact may be published without valid application and installer signatures.'
    Assert-True ($result.ExitCode -eq 1 -and $result.Error -ceq $expected) 'CLI tag must reject the exact missing configuration.'
    Assert-True (-not (Test-Path -LiteralPath $result.EnvironmentFile)) 'Rejected tag must not enable signing.'

    foreach ($eventName in @('pull_request', 'workflow_dispatch')) {
        $result = Invoke-SigningCli 'branch' $eventName $configured
        Assert-True ($result.ExitCode -eq 0 -and $result.Error -eq '') 'Branch CLI must succeed without errors.'
        Assert-True ((Get-Content -LiteralPath $result.EnvironmentFile -Raw).Trim() -ceq 'SIGN=false') 'Branch CLI must disable signing in GITHUB_ENV.'
        Assert-True ($result.Output -ceq 'Unsigned test artifacts only; this run cannot create a release.') 'Branch CLI must describe test artifacts precisely.'
    }
    foreach ($artifact in @($application, $installer)) {
        $result = Invoke-SigningCli 'tag' 'push' @{} -Action VerifyArtifacts -Artifact $artifact
        Assert-True ($result.ExitCode -eq 1 -and
            $result.Error -ceq "Release artifact does not have a valid Authenticode signature: $artifact") 'CLI must reject the exact real unsigned artifact.'
        Assert-True (-not (Test-Path -LiteralPath $result.EnvironmentFile)) 'Artifact verification must not change the signing environment.'
    }
}
finally {
    # Only these known fixture files are removed; no recursive cleanup of a computed path.
    Remove-Item -LiteralPath $application, $installer
    foreach ($file in $cliEnvironmentFiles) {
        if (Test-Path -LiteralPath $file) { Remove-Item -LiteralPath $file }
    }
    Remove-Item -LiteralPath $testRoot
}
Write-Output "Release signing checks: $passed passed."
