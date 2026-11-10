# Génère assets\icon.ico (multi-tailles) et assets\icon.png (256 px) pour ImageSort.
# Concept : une photo qui glisse dans un dossier, avec une touche de clavier « A » en badge
# (on trie à coups de touches). Dessin vectoriel via System.Drawing, rendu à chaque taille.
#
#   powershell -ExecutionPolicy Bypass -File tools\make_icon.ps1

Add-Type -AssemblyName System.Drawing
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$out = Join-Path $root 'assets'
New-Item -ItemType Directory $out -Force | Out-Null

function RoundRect([float]$x, [float]$y, [float]$w, [float]$h, [float]$r) {
    $p = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $p.AddArc($x, $y, $d, $d, 180, 90)
    $p.AddArc($x + $w - $d, $y, $d, $d, 270, 90)
    $p.AddArc($x + $w - $d, $y + $h - $d, $d, $d, 0, 90)
    $p.AddArc($x, $y + $h - $d, $d, $d, 90, 90)
    $p.CloseFigure()
    return $p
}

function Hex([string]$h, [int]$a = 255) {
    [System.Drawing.Color]::FromArgb($a, [Convert]::ToInt32($h.Substring(1, 2), 16), [Convert]::ToInt32($h.Substring(3, 2), 16), [Convert]::ToInt32($h.Substring(5, 2), 16))
}

function LinGrad([float]$x, [float]$y, [float]$w, [float]$h, [string]$c1, [string]$c2, [float]$angle) {
    $rect = New-Object System.Drawing.RectangleF($x, $y, $w, $h)
    New-Object System.Drawing.Drawing2D.LinearGradientBrush($rect, (Hex $c1), (Hex $c2), $angle)
}

function Draw([int]$s) {
    $bmp = New-Object System.Drawing.Bitmap $s, $s, ([System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.InterpolationMode = 'HighQualityBicubic'
    $g.PixelOffsetMode = 'HighQuality'
    $g.TextRenderingHint = 'AntiAliasGridFit'
    $g.Clear([System.Drawing.Color]::Transparent)
    $g.ScaleTransform($s / 256.0, $s / 256.0)

    # 1. Fond : carré arrondi dégradé bleu -> violet
    $bgPath = RoundRect 6 6 244 244 54
    $g.FillPath((LinGrad 6 6 244 244 '#2563EB' '#7C3AED' 45), $bgPath)
    # léger reflet en haut
    $g.SetClip($bgPath)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush (Hex '#FFFFFF' 28)), -40, -120, 340, 220)
    $g.ResetClip()

    # 2. Dossier (arrière + onglet)
    $backBrush = New-Object System.Drawing.SolidBrush (Hex '#D97706')
    $g.FillPath($backBrush, (RoundRect 36 96 90 40 12))   # onglet
    $g.FillPath($backBrush, (RoundRect 36 112 184 100 16)) # corps arrière

    # 3. Photo inclinée qui « entre » dans le dossier
    $state = $g.Save()
    $g.TranslateTransform(130, 128)
    $g.RotateTransform(-11)
    $g.TranslateTransform(-130, -128)
    $g.FillPath((New-Object System.Drawing.SolidBrush (Hex '#000000' 45)), (RoundRect 82 40 108 96 9))  # ombre portée (décalée)
    $g.FillPath((New-Object System.Drawing.SolidBrush (Hex '#FFFFFF')), (RoundRect 78 34 108 96 9))
    $imgPath = RoundRect 85 41 94 70 4
    $g.FillPath((LinGrad 85 41 94 70 '#7DD3FC' '#38BDF8' 90), $imgPath)
    $g.SetClip($imgPath)
    $g.FillEllipse((New-Object System.Drawing.SolidBrush (Hex '#FDE047')), 142, 48, 24, 24)  # soleil
    $mtn1 = [System.Drawing.PointF[]]@((New-Object System.Drawing.PointF(80, 114)), (New-Object System.Drawing.PointF(114, 70)), (New-Object System.Drawing.PointF(150, 114)))
    $mtn2 = [System.Drawing.PointF[]]@((New-Object System.Drawing.PointF(120, 114)), (New-Object System.Drawing.PointF(150, 82)), (New-Object System.Drawing.PointF(185, 114)))
    $g.FillPolygon((New-Object System.Drawing.SolidBrush (Hex '#15803D')), $mtn2)
    $g.FillPolygon((New-Object System.Drawing.SolidBrush (Hex '#22C55E')), $mtn1)
    $g.ResetClip()
    $g.Restore($state)

    # 4. Devant du dossier (recouvre le bas de la photo)
    $front = RoundRect 36 138 184 76 16
    $g.FillPath((New-Object System.Drawing.SolidBrush (Hex '#000000' 40)), (RoundRect 36 134 184 76 16)) # ombre sur la photo
    $g.FillPath((LinGrad 36 134 184 80 '#FCD34D' '#F59E0B' 90), $front)
    $g.FillRectangle((New-Object System.Drawing.SolidBrush (Hex '#FFFFFF' 70)), 52, 142, 152, 3) # liseré lumineux

    # 5. Badge « touche A » (seulement si assez grand pour être lisible)
    if ($s -ge 48) {
        $keyPath = RoundRect 150 156 70 66 14
        $g.FillPath((New-Object System.Drawing.SolidBrush (Hex '#000000' 70)), (RoundRect 150 162 70 66 14))
        $g.FillPath((LinGrad 150 156 70 66 '#FFFFFF' '#E2E8F0' 90), $keyPath)
        $font = New-Object System.Drawing.Font('Segoe UI', 40, [System.Drawing.FontStyle]::Bold, [System.Drawing.GraphicsUnit]::Pixel)
        $sf = New-Object System.Drawing.StringFormat
        $sf.Alignment = 'Center'; $sf.LineAlignment = 'Center'
        $g.DrawString('A', $font, (New-Object System.Drawing.SolidBrush (Hex '#4C1D95')), (New-Object System.Drawing.RectangleF(150, 154, 70, 66)), $sf)
    }
    $g.Dispose()
    return $bmp
}

# Entrée d'icône au format DIB 32 bits (BGRA, de bas en haut) + masque AND : le format le plus
# compatible pour les petites tailles. Seule la taille 256 est stockée en PNG.
function DibEntry([System.Drawing.Bitmap]$bmp) {
    $w = $bmp.Width; $h = $bmp.Height
    $rect = New-Object System.Drawing.Rectangle(0, 0, $w, $h)
    $data = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::ReadOnly, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $pixels = New-Object byte[] ($w * $h * 4)
    [System.Runtime.InteropServices.Marshal]::Copy($data.Scan0, $pixels, 0, $pixels.Length)
    $bmp.UnlockBits($data)
    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter $ms
    $bw.Write([uint32]40); $bw.Write([int32]$w); $bw.Write([int32]($h * 2))
    $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]0)
    $bw.Write([uint32]($w * $h * 4)); $bw.Write([int32]0); $bw.Write([int32]0); $bw.Write([uint32]0); $bw.Write([uint32]0)
    for ($y = $h - 1; $y -ge 0; $y--) { $bw.Write($pixels, $y * $w * 4, $w * 4) }
    $maskRow = [int]([Math]::Ceiling($w / 32.0) * 4)
    $bw.Write((New-Object byte[] ($maskRow * $h)))
    $bw.Flush()
    return , ([byte[]]$ms.ToArray())   # la virgule empêche PowerShell de « dérouler » le tableau
}

$sizes = 16, 24, 32, 48, 64, 128, 256
$pngs = @{}
foreach ($s in $sizes) {
    $bmp = Draw $s
    if ($s -ge 256) {
        $ms = New-Object System.IO.MemoryStream
        $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
        $pngs[$s] = $ms.ToArray()
        $bmp.Save((Join-Path $out 'icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    } else {
        $pngs[$s] = [byte[]](DibEntry $bmp)
    }
    $bmp.Dispose()
}

# Assemblage du .ico
$fs = [System.IO.File]::Create((Join-Path $out 'icon.ico'))
$bw = New-Object System.IO.BinaryWriter $fs
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$sizes.Count)
$offset = 6 + 16 * $sizes.Count
foreach ($s in $sizes) {
    $dim = if ($s -ge 256) { 0 } else { $s }
    $bw.Write([byte]$dim); $bw.Write([byte]$dim); $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$pngs[$s].Length); $bw.Write([uint32]$offset)
    $offset += $pngs[$s].Length
}
foreach ($s in $sizes) { $bw.Write($pngs[$s]) }
$bw.Close(); $fs.Close()
"OK : $out\icon.ico, $out\icon.png"
