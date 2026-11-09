# 06: Çalışma akışı

## Derleme ve test

`cargo` bu makinede bash PATH'inde değildir; tam yolu kullan:

```bash
cd yeniproje
~/.cargo/bin/cargo test
~/.cargo/bin/cargo build --release
```

- Her değişiklikten sonra ikisini de çalıştır. Testlerin hepsi geçmeli; derleme uyarısız olmalı.
- Release sürümü `windows_subsystem = "windows"` ile derlenir: konsol açılmaz. Komut satırı alt komutları (`help`, `run`...) `AttachConsole` ile terminale yazar.
- Bazı testler oyun kuruluysa gerçek dosyaları okur (`items.dat`, `.rttex`, `../world.dat`); dosya yoksa kendini atlar.

## Pencereyi görsel olarak kontrol etme

Geliştirme sürümünde **önizleme modu** var: proxy çalışmadan, örnek verilerle bir sayfa açar.

| Ortam değişkeni | Etki |
|---|---|
| `GTPROXY_PREVIEW=<0-5>` | 0 Genel bakış, 1 Özellikler, 2 Dünya, 3 Rulet, 4 Log, 5 Ayarlar |
| `GTPROXY_PREVIEW_SCENE=title\|connecting\|plain` | arka plan sahnesi |
| `GTPROXY_SIZE=<g>x<y>` | başlangıç pencere boyutu (ör. `780x560` dar, `1180x780` geniş) |
| `GTPROXY_EMBED=1` | önizlemeyi gömmeli mod düzeninde açar (oyun aranmaz); `=panel` sayfa paneli açık halde |
| `GTPROXY_SHOT=<dosya.bmp>` | önizleme 30 kare sonra **kendi ekran görüntüsünü** BMP olarak kaydeder ve kapanır |

Ekran görüntüsü için **`GTPROXY_SHOT` kullan** (ekran yakalama DPI yüzünden yanlış bölgeyi alıyor):
1. `GTPROXY_PREVIEW`, `GTPROXY_SIZE`, `GTPROXY_SHOT` ortam değişkenlerini ayarla ve `target\debug\gtproxy.exe`'yi başlat; süreç kendisi kapanır.
2. BMP'yi PNG'ye çevir (Python PIL) ve Read aracıyla incele.
3. Düzen değişikliklerini en az bir dar (`780x560`) ve bir geniş (`1180x780`) boyutta kontrol et.

## Gerçek oyunda test (kullanıcı yapar)

Ajan oyunu çalıştıramaz ve hosts/sertifika değiştiremez. Akış şöyle:
1. Ajan değişikliği yapar, test eder ve release derler.
2. Kullanıcıya ne deneyeceğini kısa maddelerle söyler. Önce güvenli/değersiz eşyalarla, küçük miktarlarla denemesini ister.
3. Kullanıcı `target\release\gtproxy.exe` → **Başlat** ile dener ve "Denedim, logu oku" der.
4. Ajan `yeniproje/gtproxy.log` dosyasını okur (`grep` ile önemli satırlar: `command:`, `ERROR`, `WARN`, ilgili paket adları). Ayrıca hosts dosyasının temiz kaldığını kontrol eder:

   ```bash
   grep -c growtopia /c/Windows/System32/drivers/etc/hosts
   ```

   Sonuç `0` olmalı.

Yeni bir paket akışını keşfetmek için kullanıcıya `/log game on` yazdırmak game paketlerini loglar. Varsayılan logda `variant` ve `text` açık, `game_packet` kapalıdır. Uzun string'ler logda 200 karakterde kesilir; tam markup gerekiyorsa ilgili yerde `tracing::debug!` ile tam metni yaz.

## Önemli yollar

| Ne | Yol |
|---|---|
| Proje | `C:\Users\serha\OneDrive\Belgeler\GitHub\GrowBots\yeniproje` |
| Oyun | `%LOCALAPPDATA%\Growtopia\Growtopia.exe` |
| Oyun önbelleği | `%LOCALAPPDATA%\Growtopia\cache\items.dat` |
| Oyun dokuları | `%LOCALAPPDATA%\Growtopia\game\*.rttex` |
| Oyun fontu | `%LOCALAPPDATA%\Growtopia\GameData\UI\Font\Century-Gothic-Bold.ttf` |
| hosts | `C:\Windows\System32\drivers\etc\hosts` |
| Referans: GrowBots | `../src` (Rust bot), `../web` (rttex çözücü vb.) |
| Referans: C++ proxy'ler | `../proxy/GTProxy-develop`, `../proxy/GrowtopiaProxy-Code` |

## Referanslardan bilgi çıkarma

- **GrowBots `src/bot/core.rs`**: login, hareket (`walk`), drop/trash, inventory, tile değişiklikleri.
- **GrowBots `src/world/mod_impl.rs`**: harita ve tile ekstraları.
- **GrowBots `src/astar.rs`**: yol bulma.
- **GrowtopiaProxy-Code `Source/Network/...`**: komut akışları (`/cd`, `/change`, spin), dialog markup örnekleri.
- **GTProxy `src/`**: temiz mimari, `OnNameChanged`/`OnChangeSkin`, text son bayt kuralı.
