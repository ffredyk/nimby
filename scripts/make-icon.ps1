<#
.SYNOPSIS
 Generates the committed Nimby branding assets: app icon PNGs, a multi-size ICO and
 the sized author mark.
.DESCRIPTION
 Pure PowerShell + System.Drawing. No external tools, no downloads, no network.
 The script is idempotent - re-running it always re-renders the same files from the
 committed sources in assets/branding.

 Outputs (written to -Out, default assets/branding):
 nimby-512.png rounded-square app icon, transparent outside the plate
 nimby-256.png rounded-square app icon, transparent outside the plate
 nimby.ico ICONDIR with 256, 128, 64, 48, 32, 24 and 16 px entries
 ffredyk-64.png author mark, transparent padding trimmed, padded to a square
 ffredyk-128.png author mark, transparent padding trimmed, padded to a square

.PARAMETER Source
 Product icon render. Default: assets/branding/nimby-icon.jpg
.PARAMETER Out
 Output directory. Default: assets/branding
.PARAMETER MarkSource
 Author mark (transparent background). Default: assets/branding/ffredyk-logo.png
.EXAMPLE
 powershell -ExecutionPolicy Bypass -File scripts/make-icon.ps1
.EXAMPLE
 powershell -File scripts/make-icon.ps1 -Source .\render.jpg -Out .\out
#>
[CmdletBinding()]
param(
 [string]$Source,
 [string]$Out,
 [string]$MarkSource
)

Set-StrictMode -Version 2.0
$ErrorActionPreference = 'Stop'

Add-Type -AssemblyName System.Drawing

$repoRoot = Split-Path -Parent $PSScriptRoot
if (-not $Source) { $Source = Join-Path $repoRoot 'assets\branding\nimby-icon.jpg' }
if (-not $MarkSource) { $MarkSource = Join-Path $repoRoot 'assets\branding\ffredyk-logo.png' }
if (-not $Out) { $Out = Join-Path $repoRoot 'assets\branding' }

$Source = [System.IO.Path]::GetFullPath($Source)
$MarkSource = [System.IO.Path]::GetFullPath($MarkSource)
$Out = [System.IO.Path]::GetFullPath($Out)

if (-not (Test-Path -LiteralPath $Source)) { throw ('Source image not found: ' + $Source) }
if (-not (Test-Path -LiteralPath $MarkSource)) { throw ('Author mark not found: ' + $MarkSource) }
if (-not (Test-Path -LiteralPath $Out)) { New-Item -ItemType Directory -Path $Out -Force | Out-Null }
# --- GDI+ helpers ------------------------------------------------------------

function New-ArgbBitmap {
 param([int]$Width, [int]$Height)
 return New-Object System.Drawing.Bitmap($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
}

function Set-HighQuality {
 param([System.Drawing.Graphics]$Graphics)
 $Graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
 $Graphics.CompositingQuality = [System.Drawing.Drawing2D.CompositingQuality]::HighQuality
 $Graphics.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
 $Graphics.PixelOffsetMode = [System.Drawing.Drawing2D.PixelOffsetMode]::HighQuality
 $Graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
}

function Load-SourceBitmap {
 param([string]$Path)
 try {
 return New-Object System.Drawing.Bitmap($Path)
 }
 catch {
 # GDI+ occasionally refuses renders whose embedded colour profile it dislikes -
 # decode through WPF and hand the pixels back to System.Drawing.
 Write-Verbose ('System.Drawing could not read ' + $Path + ': ' + $_.Exception.Message)
 Add-Type -AssemblyName PresentationCore
 $image = New-Object System.Windows.Media.Imaging.BitmapImage
 $image.BeginInit()
 $image.CacheOption = [System.Windows.Media.Imaging.BitmapCacheOption]::OnLoad
 $image.CreateOptions = [System.Windows.Media.Imaging.BitmapCreateOptions]::IgnoreColorProfile
 $image.UriSource = New-Object System.Uri($Path)
 $image.EndInit()
 $encoder = New-Object System.Windows.Media.Imaging.PngBitmapEncoder
 $encoder.Frames.Add([System.Windows.Media.Imaging.BitmapFrame]::Create($image))
 $stream = New-Object System.IO.MemoryStream
 $encoder.Save($stream)
 $stream.Position = 0
 $decoded = [System.Drawing.Image]::FromStream($stream)
 $bitmap = New-ArgbBitmap $decoded.Width $decoded.Height
 $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
 $graphics.CompositingMode = [System.Drawing.Drawing2D.CompositingMode]::SourceCopy
 $graphics.DrawImageUnscaled($decoded, 0, 0)
 $graphics.Dispose()
 $decoded.Dispose()
 $stream.Dispose()
 return $bitmap
 }
}

function Read-Pixels {
 param([System.Drawing.Bitmap]$Bitmap)
 $rect = New-Object System.Drawing.Rectangle(0, 0, $Bitmap.Width, $Bitmap.Height)
 $data = $Bitmap.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
 try {
 $buffer = New-Object byte[] ($data.Stride * $Bitmap.Height)
 [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $buffer, 0, $buffer.Length)
 return @{ Buffer = $buffer; Stride = $data.Stride }
 }
 finally {
 $Bitmap.UnlockBits($data)
 }
}

function Write-Pixels {
 param([System.Drawing.Bitmap]$Bitmap, [byte[]]$Buffer)
 $rect = New-Object System.Drawing.Rectangle(0, 0, $Bitmap.Width, $Bitmap.Height)
 $data = $Bitmap.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
 try {
 [System.Runtime.InteropServices.Marshal]::Copy($Buffer, 0, $data.Scan0, $Buffer.Length)
 }
 finally {
 $Bitmap.UnlockBits($data)
 }
}
function New-ScaledBitmap {
 param([System.Drawing.Bitmap]$Bitmap, [int]$Width, [int]$Height)
 # halve repeatedly so big reductions stay clean, then take the last step
 $current = $Bitmap
 $owned = $false
 while (($current.Width -gt $Width * 2) -and ($current.Height -gt $Height * 2)) {
 $nextWidth = [int][math]::Max($Width, [math]::Ceiling($current.Width / 2.0))
 $nextHeight = [int][math]::Max($Height, [math]::Ceiling($current.Height / 2.0))
 $step = New-ArgbBitmap $nextWidth $nextHeight
 $graphics = [System.Drawing.Graphics]::FromImage($step)
 Set-HighQuality $graphics
 $graphics.Clear([System.Drawing.Color]::Transparent)
 $graphics.DrawImage($current, (New-Object System.Drawing.Rectangle(0, 0, $nextWidth, $nextHeight)))
 $graphics.Dispose()
 if ($owned) { $current.Dispose() }
 $current = $step
 $owned = $true
 }
 if (($current.Width -eq $Width) -and ($current.Height -eq $Height)) {
 if ($owned) { return $current }
 return [System.Drawing.Bitmap]$Bitmap.Clone()
 }
 $result = New-ArgbBitmap $Width $Height
 $graphics = [System.Drawing.Graphics]::FromImage($result)
 Set-HighQuality $graphics
 $graphics.Clear([System.Drawing.Color]::Transparent)
 $graphics.DrawImage($current, (New-Object System.Drawing.Rectangle(0, 0, $Width, $Height)))
 $graphics.Dispose()
 if ($owned) { $current.Dispose() }
 return $result
}

function New-RoundedRectPath {
 param([double]$Side, [double]$Radius)
 $r = [single]$Radius
 if ($r -le 0) { $r = 0.5 }
 $d = $r * 2
 $max = [single]$Side
 $path = New-Object System.Drawing.Drawing2D.GraphicsPath
 $path.AddArc([single]0, [single]0, $d, $d, 180, 90)
 $path.AddArc([single]($max - $d), [single]0, $d, $d, 270, 90)
 $path.AddArc([single]($max - $d), [single]($max - $d), $d, $d, 0, 90)
 $path.AddArc([single]0, [single]($max - $d), $d, $d, 90, 90)
 $path.CloseFigure()
 return $path
}

function Get-RoundedMask {
 param([int]$Size, [double]$RadiusPercent)
 # 4x supersampling keeps the corners smooth at every target size
 $large = $Size * 4
 $mask = New-ArgbBitmap $large $large
 $graphics = [System.Drawing.Graphics]::FromImage($mask)
 $graphics.Clear([System.Drawing.Color]::Transparent)
 $graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
 $path = New-RoundedRectPath $large ($large * $RadiusPercent)
 $brush = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::White)
 $graphics.FillPath($brush, $path)
 $brush.Dispose()
 $path.Dispose()
 $graphics.Dispose()
 $small = New-ScaledBitmap $mask $Size $Size
 $mask.Dispose()
 return $small
}

function Merge-AlphaMask {
 param([System.Drawing.Bitmap]$Bitmap, [System.Drawing.Bitmap]$Mask)
 $source = Read-Pixels $Bitmap
 $maskPixels = Read-Pixels $Mask
 $buffer = $source['Buffer']
 $stride = $source['Stride']
 $maskBuffer = $maskPixels['Buffer']
 $maskStride = $maskPixels['Stride']
 for ($y = 0; $y -lt $Bitmap.Height; $y++) {
 $rowOffset = $y * $stride
 $maskOffset = $y * $maskStride
 for ($x = 0; $x -lt $Bitmap.Width; $x++) {
 $i = $rowOffset + $x * 4 + 3
 $alpha = $buffer[$i]
 $m = $maskBuffer[$maskOffset + $x * 4 + 3]
 if ($m -ne 255) {
 if ($m -eq 0) { $alpha = 0 }
 else { $alpha = [byte](($alpha * $m + 127) / 255) }
 $buffer[$i] = $alpha
 }
 if ($alpha -eq 0) {
 # keep fully transparent pixels black so nothing bleeds at the plate edge
 $j = $i - 3
 $buffer[$j] = 0
 $buffer[$j + 1] = 0
 $buffer[$j + 2] = 0
 }
 }
 }
 Write-Pixels $Bitmap $buffer
}
function Invoke-Sharpen {
 param([System.Drawing.Bitmap]$Bitmap, [double]$Amount)
 if ($Amount -le 0) { return }
 $pixels = Read-Pixels $Bitmap
 $buffer = $pixels['Buffer']
 $stride = $pixels['Stride']
 $result = New-Object byte[] $buffer.Length
 [System.Array]::Copy($buffer, $result, $buffer.Length)
 $width = $Bitmap.Width
 $height = $Bitmap.Height
 for ($y = 1; $y -lt $height - 1; $y++) {
 $rowOffset = $y * $stride
 for ($x = 1; $x -lt $width - 1; $x++) {
 $i = $rowOffset + $x * 4
 # only touch fully opaque pixels so the plate edge keeps its clean antialiasing
 if ($buffer[$i + 3] -ne 255) { continue }
 for ($c = 0; $c -lt 3; $c++) {
 $centre = [int]$buffer[$i + $c]
 $cross = [int]$buffer[$i - 4 + $c] + [int]$buffer[$i + 4 + $c] + [int]$buffer[$i - $stride + $c] + [int]$buffer[$i + $stride + $c]
 $value = $centre + ($Amount * (4 * $centre - $cross)) / 4.0
 if ($value -lt 0) { $value = 0 }
 if ($value -gt 255) { $value = 255 }
 $result[$i + $c] = [byte]$value
 }
 }
 }
 Write-Pixels $Bitmap $result
}

function Get-ContentBounds {
 param([System.Drawing.Bitmap]$Bitmap, [System.Drawing.Color]$PlateColor, [int]$Tolerance = 48)
 $pixels = Read-Pixels $Bitmap
 $buffer = $pixels['Buffer']
 $stride = $pixels['Stride']
 $width = $Bitmap.Width
 $height = $Bitmap.Height
 $minX = $width
 $minY = $height
 $maxX = -1
 $maxY = -1
 $limit = $Tolerance * $Tolerance
 for ($y = 0; $y -lt $height; $y++) {
 $rowOffset = $y * $stride
 for ($x = 0; $x -lt $width; $x++) {
 $i = $rowOffset + $x * 4
 $db = [int]$buffer[$i] - $PlateColor.B
 $dg = [int]$buffer[$i + 1] - $PlateColor.G
 $dr = [int]$buffer[$i + 2] - $PlateColor.R
 if (($dr * $dr + $dg * $dg + $db * $db) -gt $limit) {
 if ($x -lt $minX) { $minX = $x }
 if ($x -gt $maxX) { $maxX = $x }
 if ($y -lt $minY) { $minY = $y }
 if ($y -gt $maxY) { $maxY = $y }
 }
 }
 }
 if ($maxX -lt 0) { return (New-Object System.Drawing.Rectangle(0, 0, $width, $height)) }
 $boundsWidth = $maxX - $minX + 1
 $boundsHeight = $maxY - $minY + 1
 return (New-Object System.Drawing.Rectangle($minX, $minY, $boundsWidth, $boundsHeight))
}

function Get-PlateColor {
 param([System.Drawing.Bitmap]$Bitmap)
 $inset = [int][math]::Max(2, [math]::Round([math]::Min($Bitmap.Width, $Bitmap.Height) * 0.02))
 $samples = 32
 $sumR = 0.0
 $sumG = 0.0
 $sumB = 0.0
 $count = 0
 for ($k = 0; $k -lt $samples; $k++) {
 $x = [int][math]::Round(($Bitmap.Width - 1 - 2 * $inset) * $k / ($samples - 1.0)) + $inset
 $y = [int][math]::Round(($Bitmap.Height - 1 - 2 * $inset) * $k / ($samples - 1.0)) + $inset
 $colours = @(
 $Bitmap.GetPixel($x, $inset),
 $Bitmap.GetPixel($x, $Bitmap.Height - 1 - $inset),
 $Bitmap.GetPixel($inset, $y),
 $Bitmap.GetPixel($Bitmap.Width - 1 - $inset, $y)
 )
 foreach ($colour in $colours) {
 $sumR += $colour.R
 $sumG += $colour.G
 $sumB += $colour.B
 $count++
 }
 }
 return [System.Drawing.Color]::FromArgb(255, [int][math]::Round($sumR / $count), [int][math]::Round($sumG / $count), [int][math]::Round($sumB / $count))
}
function Get-AlphaBounds {
 param([System.Drawing.Bitmap]$Bitmap, [int]$Threshold = 8)
 $pixels = Read-Pixels $Bitmap
 $buffer = $pixels['Buffer']
 $stride = $pixels['Stride']
 $minX = $Bitmap.Width
 $minY = $Bitmap.Height
 $maxX = -1
 $maxY = -1
 for ($y = 0; $y -lt $Bitmap.Height; $y++) {
 $rowOffset = $y * $stride
 for ($x = 0; $x -lt $Bitmap.Width; $x++) {
 if ($buffer[$rowOffset + $x * 4 + 3] -gt $Threshold) {
 if ($x -lt $minX) { $minX = $x }
 if ($x -gt $maxX) { $maxX = $x }
 if ($y -lt $minY) { $minY = $y }
 if ($y -gt $maxY) { $maxY = $y }
 }
 }
 }
 if ($maxX -lt 0) { return (New-Object System.Drawing.Rectangle(0, 0, $Bitmap.Width, $Bitmap.Height)) }
 $boundsWidth = $maxX - $minX + 1
 $boundsHeight = $maxY - $minY + 1
 return (New-Object System.Drawing.Rectangle($minX, $minY, $boundsWidth, $boundsHeight))
}
function ConvertTo-ArgbBitmap {
 param([System.Drawing.Bitmap]$Bitmap)
 if ($Bitmap.PixelFormat -eq [System.Drawing.Imaging.PixelFormat]::Format32bppArgb) { return $Bitmap }
 $copy = New-ArgbBitmap $Bitmap.Width $Bitmap.Height
 $graphics = [System.Drawing.Graphics]::FromImage($copy)
 Set-HighQuality $graphics
 $graphics.Clear([System.Drawing.Color]::Transparent)
 $graphics.DrawImageUnscaled($Bitmap, 0, 0)
 $graphics.Dispose()
 return $copy
}

function New-IconBitmap {
 param(
 [System.Drawing.Bitmap]$Bitmap,
 [int]$Size,
 [System.Drawing.Rectangle]$SourceRect,
 [double]$RadiusPercent,
 [double]$SharpenAmount,
 [System.Drawing.Color]$PlateColor
 )
 $crop = $Bitmap.Clone($SourceRect, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
 $scaled = New-ScaledBitmap $crop $Size $Size
 $crop.Dispose()
 $icon = New-ArgbBitmap $Size $Size
 $graphics = [System.Drawing.Graphics]::FromImage($icon)
 Set-HighQuality $graphics
 $graphics.Clear($PlateColor)
 $graphics.DrawImageUnscaled($scaled, 0, 0)
 $graphics.Dispose()
 $scaled.Dispose()
 $mask = Get-RoundedMask $Size $RadiusPercent
 Merge-AlphaMask $icon $mask
 $mask.Dispose()
 Invoke-Sharpen $icon $SharpenAmount
 return $icon
}

function New-MarkBitmap {
 param([System.Drawing.Bitmap]$Bitmap, [int]$Size)
 $bounds = Get-AlphaBounds $Bitmap
 $side = [int][math]::Max($bounds.Width, $bounds.Height)
 $square = New-ArgbBitmap $side $side
 $graphics = [System.Drawing.Graphics]::FromImage($square)
 Set-HighQuality $graphics
 $graphics.Clear([System.Drawing.Color]::Transparent)
 $destX = [int][math]::Round(($side - $bounds.Width) / 2.0)
 $destY = [int][math]::Round(($side - $bounds.Height) / 2.0)
 $destination = New-Object System.Drawing.Rectangle($destX, $destY, $bounds.Width, $bounds.Height)
 $graphics.DrawImage($Bitmap, $destination, $bounds, [System.Drawing.GraphicsUnit]::Pixel)
 $graphics.Dispose()
 $result = New-ScaledBitmap $square $Size $Size
 $square.Dispose()
 return $result
}

function Save-Png {
 param([System.Drawing.Bitmap]$Bitmap, [string]$Path)
 $Bitmap.Save($Path, [System.Drawing.Imaging.ImageFormat]::Png)
}

function Get-PngBytes {
 param([System.Drawing.Bitmap]$Bitmap)
 $stream = New-Object System.IO.MemoryStream
 try {
 $Bitmap.Save($stream, [System.Drawing.Imaging.ImageFormat]::Png)
 return $stream.ToArray()
 }
 finally {
 $stream.Dispose()
 }
}

function Get-DibBytes {
 param([System.Drawing.Bitmap]$Bitmap)
 $width = $Bitmap.Width
 $height = $Bitmap.Height
 $pixels = Read-Pixels $Bitmap
 $buffer = $pixels['Buffer']
 $stride = $pixels['Stride']
 $stream = New-Object System.IO.MemoryStream
 $writer = New-Object System.IO.BinaryWriter($stream)
 try {
 # BITMAPINFOHEADER, height doubled because the AND mask follows the pixels
 $writer.Write([UInt32]40)
 $writer.Write([Int32]$width)
 $writer.Write([Int32]($height * 2))
 $writer.Write([UInt16]1)
 $writer.Write([UInt16]32)
 $writer.Write([UInt32]0)
 $writer.Write([UInt32]($width * $height * 4))
 $writer.Write([Int32]0)
 $writer.Write([Int32]0)
 $writer.Write([UInt32]0)
 $writer.Write([UInt32]0)
 for ($y = $height - 1; $y -ge 0; $y--) {
 $writer.Write($buffer, $y * $stride, $width * 4)
 }
 # 1bpp AND mask, all zero: transparency comes from the alpha channel
 $maskStride = [int]([math]::Floor(($width + 31) / 32) * 4)
 $mask = New-Object byte[] ($maskStride * $height)
 $writer.Write($mask, 0, $mask.Length)
 $writer.Flush()
 return $stream.ToArray()
 }
 finally {
 $writer.Dispose()
 $stream.Dispose()
 }
}

function Write-IcoFile {
 param([string]$Path, [object[]]$Entries)
 $stream = New-Object System.IO.MemoryStream
 $writer = New-Object System.IO.BinaryWriter($stream)
 try {
 $writer.Write([UInt16]0)
 $writer.Write([UInt16]1)
 $writer.Write([UInt16]$Entries.Length)
 $offset = 6 + 16 * $Entries.Length
 foreach ($entry in $Entries) {
 $size = [int]$entry['Size']
 $bytes = [byte[]]$entry['Bytes']
 $dimension = [byte]0
 if ($size -lt 256) { $dimension = [byte]$size }
 $writer.Write($dimension)
 $writer.Write($dimension)
 $writer.Write([byte]0)
 $writer.Write([byte]0)
 $writer.Write([UInt16]1)
 $writer.Write([UInt16]32)
 $writer.Write([UInt32]$bytes.Length)
 $writer.Write([UInt32]$offset)
 $offset += $bytes.Length
 }
 foreach ($entry in $Entries) {
 $bytes = [byte[]]$entry['Bytes']
 $writer.Write($bytes, 0, $bytes.Length)
 }
 $writer.Flush()
 [System.IO.File]::WriteAllBytes($Path, $stream.ToArray())
 }
 finally {
 $writer.Dispose()
 $stream.Dispose()
 }
}

# --- render ------------------------------------------------------------------

$loaded = Load-SourceBitmap $Source
$sourceBitmap = ConvertTo-ArgbBitmap $loaded
if (-not [object]::ReferenceEquals($loaded, $sourceBitmap)) { $loaded.Dispose() }

$plateColor = Get-PlateColor $sourceBitmap
$plateHex = '#' + $plateColor.R.ToString('X2') + $plateColor.G.ToString('X2') + $plateColor.B.ToString('X2')
Write-Host ('source : ' + $Source + ' (' + $sourceBitmap.Width + 'x' + $sourceBitmap.Height + ', plate ' + $plateHex + ')')

$fullRect = New-Object System.Drawing.Rectangle(0, 0, $sourceBitmap.Width, $sourceBitmap.Height)
$contentBounds = Get-ContentBounds $sourceBitmap $plateColor
Write-Host ('mark : ' + $contentBounds.Width + 'x' + $contentBounds.Height + ' at ' + $contentBounds.X + ',' + $contentBounds.Y)

# square crop around the mark so the tiny icon entries fill their plate
$tightSide = [int][math]::Round([math]::Max($contentBounds.Width, $contentBounds.Height) * 1.08)
$limit = [math]::Min($sourceBitmap.Width, $sourceBitmap.Height)
if ($tightSide -gt $limit) { $tightSide = $limit }
$tightX = [int][math]::Round($contentBounds.X + $contentBounds.Width / 2.0 - $tightSide / 2.0)
$tightY = [int][math]::Round($contentBounds.Y + $contentBounds.Height / 2.0 - $tightSide / 2.0)
if ($tightX -lt 0) { $tightX = 0 }
if ($tightY -lt 0) { $tightY = 0 }
if (($tightX + $tightSide) -gt $sourceBitmap.Width) { $tightX = $sourceBitmap.Width - $tightSide }
if (($tightY + $tightSide) -gt $sourceBitmap.Height) { $tightY = $sourceBitmap.Height - $tightSide }
$tightRect = New-Object System.Drawing.Rectangle($tightX, $tightY, $tightSide, $tightSide)

$written = @()

foreach ($size in @(512, 256)) {
 $icon = New-IconBitmap $sourceBitmap $size $fullRect 0.22 0.0 $plateColor
 $path = Join-Path $Out ('nimby-' + $size + '.png')
 Save-Png $icon $path
 $icon.Dispose()
 $written += $path
}

# 256 px is the only PNG-compressed entry - a 32-bit DIB that large would cost
# ~260 KB on its own. Everything smaller stays uncompressed DIB so that older
# consumers (for example System.Drawing.Icon.ToBitmap) can still decode it.
# 24 and 16 px are re-cropped so the mark fills the plate and keep a lighter
# corner radius; they are then sharpened slightly to survive the reduction.
$iconSpec = @(
 @{ Size = 256; Format = 'Png'; Rect = $fullRect; Radius = 0.22; Sharpen = 0.00 },
 @{ Size = 128; Format = 'Dib'; Rect = $fullRect; Radius = 0.22; Sharpen = 0.00 },
 @{ Size = 64; Format = 'Dib'; Rect = $fullRect; Radius = 0.22; Sharpen = 0.00 },
 @{ Size = 48; Format = 'Dib'; Rect = $fullRect; Radius = 0.22; Sharpen = 0.00 },
 @{ Size = 32; Format = 'Dib'; Rect = $fullRect; Radius = 0.22; Sharpen = 0.20 },
 @{ Size = 24; Format = 'Dib'; Rect = $tightRect; Radius = 0.20; Sharpen = 0.35 },
 @{ Size = 16; Format = 'Dib'; Rect = $tightRect; Radius = 0.18; Sharpen = 0.35 }
)

$entries = @()
foreach ($spec in $iconSpec) {
 $bitmap = New-IconBitmap $sourceBitmap $spec['Size'] $spec['Rect'] $spec['Radius'] $spec['Sharpen'] $plateColor
 if ($spec['Format'] -eq 'Png') { $bytes = Get-PngBytes $bitmap } else { $bytes = Get-DibBytes $bitmap }
 $bitmap.Dispose()
 $entries += @{ Size = $spec['Size']; Bytes = $bytes }
}
$icoPath = Join-Path $Out 'nimby.ico'
Write-IcoFile $icoPath $entries
$written += $icoPath

$markLoaded = Load-SourceBitmap $MarkSource
$markBitmap = ConvertTo-ArgbBitmap $markLoaded
if (-not [object]::ReferenceEquals($markLoaded, $markBitmap)) { $markLoaded.Dispose() }
foreach ($markSize in @(64, 128)) {
 $mark = New-MarkBitmap $markBitmap $markSize
 $path = Join-Path $Out ('ffredyk-' + $markSize + '.png')
 Save-Png $mark $path
 $mark.Dispose()
 $written += $path
}
$markBitmap.Dispose()
$sourceBitmap.Dispose()

Write-Host ''
foreach ($file in $written) {
 $item = Get-Item -LiteralPath $file
 Write-Host (' ' + $item.Name.PadRight(18) + $item.Length.ToString().PadLeft(8) + ' B')
}