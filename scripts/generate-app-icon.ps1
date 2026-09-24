[CmdletBinding()]
param(
    [string]$Source = (Join-Path $PSScriptRoot '..\assets\source\app-icon.png'),
    [string]$Output = (Join-Path $PSScriptRoot '..\assets\app-icon.ico'),
    [int[]]$Sizes = @(16, 24, 32, 48, 64, 128, 256)
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$sourcePath = (Resolve-Path -LiteralPath $Source).Path
$outputPath = [System.IO.Path]::GetFullPath($Output)
$normalizedSizes = @($Sizes | Sort-Object -Unique)

if ($normalizedSizes.Count -eq 0) {
    throw 'At least one icon size is required.'
}

foreach ($size in $normalizedSizes) {
    if ($size -lt 1 -or $size -gt 256) {
        throw "ICO dimensions must be between 1 and 256 pixels: $size"
    }
}

$outputDirectory = Split-Path -Parent $outputPath
if (-not (Test-Path -LiteralPath $outputDirectory)) {
    New-Item -ItemType Directory -Path $outputDirectory -Force | Out-Null
}

$sourceImage = [System.Drawing.Image]::FromFile($sourcePath)
try {
    if ($sourceImage.Width -ne $sourceImage.Height) {
        throw "The source artwork must be square, got $($sourceImage.Width)x$($sourceImage.Height)."
    }

    if ($sourceImage.Width -lt ($normalizedSizes | Measure-Object -Maximum).Maximum) {
        throw 'The source artwork is smaller than the largest requested icon size.'
    }

    $frames = foreach ($size in $normalizedSizes) {
        $bitmap = [System.Drawing.Bitmap]::new(
            $size,
            $size,
            [System.Drawing.Imaging.PixelFormat]::Format32bppArgb
        )
        try {
            $bitmap.SetResolution(96.0, 96.0)
            $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
            try {
                $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
                $graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
                $graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
                $graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
                $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::HighQuality
                $graphics.Clear([System.Drawing.Color]::Transparent)
                $destination = [System.Drawing.Rectangle]::new(0, 0, $size, $size)
                $graphics.DrawImage(
                    $sourceImage,
                    $destination,
                    0,
                    0,
                    $sourceImage.Width,
                    $sourceImage.Height,
                    [System.Drawing.GraphicsUnit]::Pixel
                )
            }
            finally {
                $graphics.Dispose()
            }

            $memory = [System.IO.MemoryStream]::new()
            try {
                $bitmap.Save($memory, [System.Drawing.Imaging.ImageFormat]::Png)
                [pscustomobject]@{
                    Size = $size
                    Bytes = [byte[]]$memory.ToArray()
                }
            }
            finally {
                $memory.Dispose()
            }
        }
        finally {
            $bitmap.Dispose()
        }
    }

    $stream = [System.IO.File]::Open(
        $outputPath,
        [System.IO.FileMode]::Create,
        [System.IO.FileAccess]::Write,
        [System.IO.FileShare]::None
    )
    try {
        $writer = [System.IO.BinaryWriter]::new($stream)
        try {
            $writer.Write([uint16]0)
            $writer.Write([uint16]1)
            $writer.Write([uint16]$frames.Count)

            $offset = 6 + (16 * $frames.Count)
            foreach ($frame in $frames) {
                $dimension = if ($frame.Size -eq 256) { [byte]0 } else { [byte]$frame.Size }
                $writer.Write($dimension)
                $writer.Write($dimension)
                $writer.Write([byte]0)
                $writer.Write([byte]0)
                $writer.Write([uint16]1)
                $writer.Write([uint16]32)
                $writer.Write([uint32]$frame.Bytes.Length)
                $writer.Write([uint32]$offset)
                $offset += $frame.Bytes.Length
            }

            foreach ($frame in $frames) {
                $writer.Write([byte[]]$frame.Bytes)
            }
        }
        finally {
            $writer.Dispose()
        }
    }
    finally {
        $stream.Dispose()
    }
}
finally {
    $sourceImage.Dispose()
}

Write-Output "Generated $outputPath with sizes: $($normalizedSizes -join ', ')"
