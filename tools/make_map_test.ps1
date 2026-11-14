# Builds captures/map_test.zip: every cell shows its own "col,row" label so button numbers can be mapped.
Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$out = "D:\My Deck App\captures\map_test.zip"
if (Test-Path $out) { Remove-Item $out }
$font = [ordered]@{ Align='bottom'; Color=16777215; FontName='Source Han Sans SC'; ShowTitle=$true; Size=10; Weight=80 }
$manifest = [ordered]@{}
$zip = [System.IO.Compression.ZipFile]::Open($out, 'Create')
foreach ($row in 0..2) { foreach ($col in 0..4) {
  if ($row -eq 2 -and $col -eq 4) { continue }               # covered by the wide slot
  $key = "${col}_${row}"
  if ($row -eq 2 -and $col -eq 3) {                           # wide clock slot, as the stock app sends it
    $manifest[$key] = [ordered]@{ SmallViewMode=1; State=0; ViewParam=@([ordered]@{ Font=$font; Text='' }) }
    continue
  }
  $bmp = New-Object System.Drawing.Bitmap 196,196
  $g = [System.Drawing.Graphics]::FromImage($bmp)
  $g.SmoothingMode = 'AntiAlias'; $g.TextRenderingHint = 'AntiAlias'
  $hue = @('#1f6feb','#2ea043','#d29922','#a371f7','#db6d28')[$col]
  $g.Clear([System.Drawing.ColorTranslator]::FromHtml($hue))
  $f = New-Object System.Drawing.Font('Segoe UI', 44, [System.Drawing.FontStyle]::Bold, 'Pixel')
  $sf = New-Object System.Drawing.StringFormat; $sf.Alignment='Center'; $sf.LineAlignment='Center'
  $g.DrawString("$col,$row", $f, [System.Drawing.Brushes]::White, (New-Object System.Drawing.RectangleF 0,0,196,196), $sf)
  $ms = New-Object System.IO.MemoryStream; $bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
  $name = "Images/map_${col}_${row}.png"
  $entry = $zip.CreateEntry($name, 'Optimal'); $es = $entry.Open(); $b = $ms.ToArray(); $es.Write($b,0,$b.Length); $es.Close()
  $g.Dispose(); $bmp.Dispose()
  $manifest[$key] = [ordered]@{ State=0; ViewParam=@([ordered]@{ Font=$font; Icon=$name; Text='' }) }
} }
$json = $manifest | ConvertTo-Json -Depth 6 -Compress
$entry = $zip.CreateEntry('manifest.json', 'Optimal'); $es = $entry.Open()
$b = [System.Text.UTF8Encoding]::new($false).GetBytes($json); $es.Write($b,0,$b.Length); $es.Close()
$zip.Dispose()
"wrote $out ($((Get-Item $out).Length) bytes)"
