<# Release signing checks shared by the workflow and its regression tests. #>
[CmdletBinding()]
param(
    [ValidateSet('CheckConfiguration', 'VerifyArtifacts')]
    [string]$Action = 'CheckConfiguration',
    [string[]]$Path
)

$ErrorActionPreference = 'Stop'

function Get-ReleaseSigningDecision {
    param([string]$RefType, [string]$EventName, [hashtable]$Configuration)

    # PR and branch builds are unsigned test artifacts, even when signing is configured.
    # Never make signing credentials available to pull-request build scripts.
    $required = $RefType -eq 'tag' -and $EventName -ne 'pull_request'
    $names = @('SIGNPATH_ORGANIZATION_ID', 'SIGNPATH_PROJECT_SLUG',
        'SIGNPATH_SIGNING_POLICY_SLUG', 'SIGNPATH_API_TOKEN')
    $missing = @($names | Where-Object { [string]::IsNullOrWhiteSpace([string]$Configuration[$_]) })
    if ($required -and $missing.Count -gt 0) {
        throw ('A tagged release requires code signing. Configure: ' + ($missing -join ', ') +
            '. No release artifact may be published without valid application and installer signatures.')
    }
    [pscustomobject]@{ Required = $required; Enabled = $required }
}

function Assert-ReleaseArtifactSignatures {
    param(
        [Parameter(Mandatory)][string[]]$LiteralPath,
        [scriptblock]$SignatureReader = { param($file) Get-AuthenticodeSignature -LiteralPath $file }
    )
    if ($LiteralPath.Count -eq 0) { throw 'At least one release artifact is required.' }
    foreach ($file in $LiteralPath) {
        if (-not (Test-Path -LiteralPath $file -PathType Leaf)) {
            throw "Release artifact is missing: $file"
        }
        $signature = & $SignatureReader $file
        if ($null -eq $signature -or [string]$signature.Status -ne 'Valid' -or
            $null -eq $signature.SignerCertificate) {
            throw "Release artifact does not have a valid Authenticode signature: $file"
        }
    }
}

if ($MyInvocation.InvocationName -ne '.') {
    if ($Action -eq 'CheckConfiguration') {
        $configuration = @{}
        foreach ($name in @('SIGNPATH_ORGANIZATION_ID', 'SIGNPATH_PROJECT_SLUG',
            'SIGNPATH_SIGNING_POLICY_SLUG', 'SIGNPATH_API_TOKEN')) {
            $configuration[$name] = [Environment]::GetEnvironmentVariable($name)
        }
        $decision = Get-ReleaseSigningDecision -RefType $env:GITHUB_REF_TYPE `
            -EventName $env:GITHUB_EVENT_NAME -Configuration $configuration
        $enabled = ([string]$decision.Enabled).ToLowerInvariant()
        if ($env:GITHUB_ENV) { "SIGN=$enabled" | Out-File -LiteralPath $env:GITHUB_ENV -Encoding utf8 -Append }
        if ($decision.Required) { Write-Output 'Release signing configuration is complete.' }
        else { Write-Output 'Unsigned test artifacts only; this run cannot create a release.' }
    }
    else {
        Assert-ReleaseArtifactSignatures -LiteralPath $Path
        Write-Output 'All release artifacts have valid Authenticode signatures.'
    }
}
