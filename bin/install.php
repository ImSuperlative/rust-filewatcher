<?php

const GITHUB_REPO = 'ImSuperlative/rust-filewatcher';
const VERSION = '0.1.0';
const BIN_DIR = __DIR__;

function detectPlatform(): array
{
    $os = php_uname('s');
    $arch = php_uname('m');

    $osMap = [
        'Linux'  => 'linux',
        'Darwin' => 'darwin',
    ];

    $archMap = [
        'x86_64'  => 'amd64',
        'amd64'   => 'amd64',
        'aarch64' => 'arm64',
        'arm64'   => 'arm64',
    ];

    $mappedOs = $osMap[$os] ?? null;
    $mappedArch = $archMap[$arch] ?? null;

    if (! $mappedOs || ! $mappedArch) {
        fwrite(STDERR, "Unsupported platform: {$os}/{$arch}\n");
        exit(1);
    }

    return [$mappedOs, $mappedArch];
}

function download(string $url, string $dest): void
{
    fwrite(STDOUT, "Downloading {$url}\n");

    if (function_exists('curl_init')) {
        $ch = curl_init($url);
        $fp = fopen($dest, 'wb');
        curl_setopt_array($ch, [
            CURLOPT_FILE            => $fp,
            CURLOPT_FOLLOWLOCATION  => true,
            CURLOPT_FAILONERROR     => true,
            CURLOPT_CONNECTTIMEOUT  => 10,
            CURLOPT_TIMEOUT         => 60,
        ]);

        $ok = curl_exec($ch);

        if (! $ok) {
            fwrite(STDERR, 'Download failed: ' . curl_error($ch) . "\n");
            fclose($fp);
            curl_close($ch);
            @unlink($dest);
            exit(1);
        }

        fclose($fp);
        curl_close($ch);

        return;
    }

    $data = @file_get_contents($url);

    if ($data === false) {
        fwrite(STDERR, "Download failed. Enable the curl extension or allow_url_fopen.\n");
        exit(1);
    }

    file_put_contents($dest, $data);
}

function verifySha256(string $binaryPath, string $checksumUrl): void
{
    $expected = @file_get_contents($checksumUrl);

    if ($expected === false) {
        fwrite(STDERR, "Warning: could not download checksum file, skipping verification.\n");
        return;
    }

    $expectedHash = trim(explode(' ', trim($expected))[0]);
    $actualHash   = hash_file('sha256', $binaryPath);

    if (! hash_equals($expectedHash, $actualHash)) {
        @unlink($binaryPath);
        fwrite(STDERR, "Checksum mismatch! Expected {$expectedHash}, got {$actualHash}\n");
        exit(1);
    }

    fwrite(STDOUT, "Checksum verified.\n");
}

[$os, $arch] = detectPlatform();

$asset    = "filewatcher-{$os}-{$arch}";
$baseUrl  = 'https://github.com/' . GITHUB_REPO . '/releases/download/v' . VERSION;
$url      = "{$baseUrl}/{$asset}";
$dest     = BIN_DIR . '/filewatcher';

download($url, $dest);
verifySha256($dest, "{$url}.sha256");
chmod($dest, 0755);

fwrite(STDOUT, "Installed {$asset} to {$dest}\n");
