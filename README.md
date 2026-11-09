# gtproxy

Growtopia için Rust ile yazılmış MITM proxy. Oyun istemcisi ile gerçek sunucu arasına girer; iki yöndeki paketleri okur, loglar, değiştirebilir ve oyun içi komutlar ekler.

```
Growtopia.exe ──HTTPS──> gtproxy :443  ──HTTPS──> www.growtopia1.com   (server_data.php)
Growtopia.exe ──ENet───> gtproxy :17091 ──ENet───> gerçek sunucu / alt sunucular
```

## Nasıl çalışır

1. **hosts:** `www.growtopia1.com`, `www.growtopia2.com`, `growtopia1.com` ve `growtopia2.com` adresleri `127.0.0.1`'e yönlendirilir. Dosyaya sadece işaretli bir blok eklenir, çıkışta sadece o blok silinir. İlk düzenlemeden önce `hosts.gtproxy.bak` adıyla bir yedek alınır.
2. **server_data.php:** Proxy, gerçek IP'yi hosts dosyasını atlayarak doğrudan `1.1.1.1` DNS sunucusuna sorar. İsteği gerçek sunucuya TLS doğrulamasıyla iletir. Cevaptaki `server|` ve `port|` alanlarını proxy adresiyle değiştirir; diğer satırlara dokunmaz.
3. **ENet köprüsü:** Proxy oyuna sunucu gibi davranır (`using_new_packet_server`), gerçek sunucuya ise istemci olarak bağlanır (`using_new_packet`). Paketler kanal ve güvenilirlik türü korunarak iletilir.
4. **Alt sunucu geçişi:** Sunucunun gönderdiği `OnSendToServer` paketinde IP ve port proxy adresiyle değiştirilir, gerçek hedef saklanır. İstemci yeniden bağlandığında proxy saklanan alt sunucuya bağlanır.

## Kurulum

Gerekenler: Rust (stable).

```bash
cargo build --release
```

İlk çalıştırmada `config.toml` ve `certs/` (yerel CA ve sunucu sertifikası) otomatik oluşturulur.

Oyun HTTPS sertifikasını doğruluyorsa CA'yı bir kez güvenilir olarak eklemen gerekir. CA, sadece mevcut Windows kullanıcısının deposuna eklenir ve Windows bir onay penceresi gösterir:

```bash
gtproxy cert install
```

Daha sonra kaldırmak için:

```bash
gtproxy cert uninstall
```

## Çalıştırma

`gtproxy.exe`'ye çift tıkla (ya da `target\release\gtproxy.exe`). Bir pencere açılır:

1. **Başlat**'a bas. Proxy hosts dosyasını düzenleyeceği için yönetici izni gerekiyorsa uygulama kendini yönetici olarak yeniden açar (Windows bir kez sorar) ve kendiliğinden başlar.
2. Proxy çalışınca **Growtopia kendiliğinden açılır** (Ayarlar'dan kapatılabilir). Normal şekilde giriş yap.
3. **Durdur**'a basınca ya da pencereyi kapatınca proxy durur ve hosts dosyası eski haline döner.

Pencerenin bölümleri:

| Bölüm | İçerik |
|---|---|
| Durum | Bağlantı ve ping, oyuncu, dünya, seviye, gem, kilit bakiyesi; herhangi bir proxy komutunu çalıştırma kutusu |
| Özellikler | Fast drop/trash, oto DL->BGL, ışınlanma ve hızı, sınırsız zoom, kıyafet gizleme, /me, formalize, spam, ping uyarısı, oto kaydet |
| Dünya | Dünyaya git / geri / relog, dünyadaki oyuncular (pull, kick, ban, isim gizle), herkese pull/kick/ban, wrench modu, kayıtlı dünyalar |
| Rulet | Kontrol aç/kapat, anında gösterme, CSN/QQ/Reme, canlı istatistik |
| Log | Canlı log, filtre |
| Ayarlar | Oyun yolu, oyunu otomatik açma, portlar, DNS, sertifika kurma/kaldırma, hosts temizleme |

Penceredeki her düğme oyun içi komutla aynı şeyi yapar; oyundaki komutlar da çalışmaya devam eder.

**Gömmeli mod** (Ayarlar → Oyun → Gömmeli mod): Growtopia'nın penceresi başlık çubuğu kaldırılıp proxy penceresinin ortasına oturtulur ve ona bağlanır. Pencere ekranı kaplar; üstte durum, dünya, warp kutusu ve hızlı anahtarlar (ışınlanma, oto toplama, fast drop), kalan alanda oyun durur. Üst çubuktaki **Panel** butonu sayfaları (Özellikler, Dünya, Rulet...) oyunun sol tarafının üstünde açar; oyun yerinden oynamaz. Panel her açılışta kapalı gelir. Oyuna bir şey enjekte edilmez, sadece penceresi taşınır. Üst çubuktaki **Normal görünüm** ya da ayar kapatılınca oyun eski haline döner. Oyunu pencere modunda kullan. Gömmeli mod tam ekran açılır. Oyun yeniden boyutlanamadığı için kendi çözünürlüğünde (Windows ölçeğiyle büyütülmüş) ortalanır; 1920x1080 ve %125 ölçekte oyunda **1280x800** seç.

**Terminal modu** (pencere istemeyenler için, arka planda çalışır): Ayarlar → Oyun → **Terminal modu**'nu açıp Kaydet'e bas ya da oyunda `/terminal` yaz (`/terminal on` / `/terminal off`); bundan sonra çift tıklayınca pencere yerine terminal açılır (gerekirse kendini yönetici olarak yeniden açar). Ya da bir terminalden:

```bash
gtproxy run
```

Terminal komut almaz: proxy arka planda çalışır, terminalde sadece log akar (penceredeki Log sayfası gibi). Komutlar oyunun içinden yazılır. Terminali kapatmak ya da Ctrl+C proxy'yi durdurur ve hosts dosyasını geri alır. Pencereye dönmek için Ayarlar'daki seçeneği `config.toml`'da `terminal = false` yap ya da `gtproxy gui` ile aç.

Proxy beklenmedik şekilde kapanırsa kalan satırları pencerenin Ayarlar bölümünden ya da şu komutla temizleyebilirsin:

```bash
gtproxy hosts remove
```

hosts dosyasını kendin yönetmek istersen Ayarlar'da "hosts dosyasını yönet"i kapat ve dört alan adını `127.0.0.1`'e yönlendir.

## Oyun içi komutlar

Komutları sohbete yazarsın. Proxy'nin tanımadığı komutlar (`/msg`, `/sb` gibi) değiştirilmeden sunucuya iletilir. Önemli uyarılar (yetersiz kilit, dünyada kimse yok vb.) sohbet yerine ekranın ortasında gösterilir.

| Komut | Açıklama |
|---|---|
| `/proxyhelp` | Tüm proxy komutlarını bir pencerede listeler (`/help` oyunun kendi komutu olarak kalır) |
| `/proxy` | Durum penceresini açar: sunucu, iki taraftaki ping, oyuncu, dünya, özellikler ve log ayarları (butonlarla açılıp kapatılabilir) |
| `/warp <dünya>` | Proxy komutu, sunucuya hiç gitmez: dünyadan çıkar, menü gelir gelmez girer (≈0.4 sn). İki warp arası en az 3 sn. `/back`, `/save`, `/list`, `/relog` ve Vend Finder da aynı yolu kullanır |
| `/back` | Önceki dünyaya döner |
| `/relog` | Bulunduğun dünyadan çıkıp tekrar girer |
| `/exit` | Dünya menüsüne çıkar |
| `/spin [csn\|qq\|reme]` | Rulet kontrol penceresini açar ya da oyun modunu seçer. Gerçek spinler anında gösterilir, başına `[REAL]`, sonuna `[QQ n]` / `[REME n]` eklenir (REAL, QQ, REME kelimeleri ve 0 yeşil, parantezler normal renkte) |
| `/fastdrop` | Açıkken drop'a basınca "kaç tane?" penceresi çıkmaz, seçili eşyanın hepsi bırakılır |
| `/fasttrash` | Aynısını trash için yapar |
| `/dw <adet>` | Envanterde yeterince varsa belirtilen sayıda World Lock bırakır |
| `/dd <adet>` | Aynısı, Diamond Lock için |
| `/db <adet>` | Aynısı, Blue Gem Lock için |
| `/daw` | Elindeki tüm World Lock, Diamond Lock ve Blue Gem Lock'ları bırakır |
| `/cd <WL değeri>` | O kadar WL değerinde kilit bırakır (örn. 250 = 2 DL + 50 WL); bozuk para yoksa DL/BGL'yi otomatik böler |
| `/balance` | Kilitlerini ve toplam WL değerini gösterir |
| `/inventory` | Envanter penceresi (ikon, isim, adet) |
| `/account` | Hesap penceresi: isim, userID, netID, seviye, gem, ülke, dünya, kilit değeri |
| `/change` | Açıkken Telephone'a wrench atınca 100 DL otomatik 1 BGL'ye çevrilir |
| `/setsave <dünya>`, `/save` | Kayıt dünyası belirler / oraya gider |
| `/wrench [pull\|kick\|ban\|off]` | Açıkken bir oyuncuya wrench atınca onu pull/kick/ban eder |
| `/pullall`, `/kickall`, `/banall` | Dünyadaki herkese uygular |
| `/spam [stop]` | Otomatik spam penceresi (iki metin, aralık en az 2 sn, rainbow) |
| `/country <kod\|off>` | Bayrağını değiştirir (sadece sende görünür) |
| `/nick [isim]` | İsmini değiştirir (sadece sende görünür); boş `/nick` gerçek ismine döndürür |
| `/skin [kod]` | Ten rengini değiştirir (sadece sende görünür); boş `/skin` gerçek rengine döndürür |
| `/ui [off]` | Oyundaki tüm pencerelerin arka plan rengini ayarlar |
| `/news` | Girişte gelen Growtopia Gazette'yi tekrar açar |
| `/debug` | Proxy iç durumu: oturum, dünyadaki oyuncular |
| `/options` | Tüm aç/kapat ayarlarının toplandığı pencere, "Save now" butonu |
| `/autosave` | Açıkken ayarlar `settings.json`'a kaydedilir ve proxy yeniden açılınca geri yüklenir |
| `/sworld [:kapı] [not]` | Bulunduğun dünyayı (istersen kapı ID'si ve notla) kaydeder |
| `/list [numara]` | Kayıtlı dünyalar penceresi (tıkla git / sil); `/list 3` doğrudan 3. dünyaya gider |
| `/hideclothes` | Diğer oyuncuların kıyafetlerini gizler (sadece sende) |
| `/hidename [isim]` | Oyuncunun isim etiketini gizler/gösterir; boş yazınca dünyadaki oyuncuları listeler |
| `/autopullpage`, `/autobanpage` | Listedeki kişi dünyana girince otomatik pull / ban; pencereden ekle-çıkar, aç-kapat |
| `/addpull <isim>`, `/addban <isim>` | Listeye hızlı ekleme; liste kapalıysa açık duruma getirir |
| `/banwords` | Dünyanda listedeki kelimeyi yazanı otomatik banlar |
| `/lp` | Son pull ettiğin oyuncuyu tekrar pull eder |
| `/me` | Açıkken yazdığın her sohbet satırı `/me` ile gider |
| `/formalize` | Sohbet satırının ilk harfini büyütür, sonuna nokta koyar |
| `/calc <ifade>` | Hesap makinesi (`+ - * / %`, parantez) |
| `/timer` | Ekranın ortasında 00:00:00 sayacı başlatır / durdurur |
| `/pingalert [ms\|off]` | Proxy–sunucu pingi bu değeri geçince ekranda uyarır |
| `/spinstats` | Rulet istatistiği: kırmızı/siyah/yeşil oranı, son spinler, QQ/Reme ortalaması |
| `/tp` | Işınlanma modunu açar/kapatır: açıkken **Ctrl basılı tutup** ekranda ulaşılabilir herhangi bir kareye tıklayınca oraya gidersin. Uzak menzil sadece Ctrl basılıyken açılır, bırakınca normal menzile döner |
| `/tp <x> <y>` | Belirtilen kareye ışınlanır |
| `/tp speed <ms>` | Yol üzerindeki her kare için bekleme (10-500 ms, varsayılan 40) |
| `/zoom` | Sınırsız zoom **varsayılan olarak açık**: oyunun kendi tekerlek zoom'u sınırsızdır. Komut açar/kapatır; değişiklik sonraki girdiğin dünyada geçerli olur (hemen için `/relog`) |
| `/zoom <seviye>` | Zoom'u doğrudan ayarlar (0.1-4, 1 = normal) |
| `/dlocal [adet]` | Önüne sadece senin ekranında görünen sahte bir Dirt düşürür (1-200); toplanamaz, sunucuya hiçbir şey gitmez |
| `/dlocal clear` | Sahte Dirt'leri kaldırır |
| `/vendfind [eşya]` | Gezdiğin dünyalardaki vending machine'leri arar: simge, isim, fiyat, görülme tarihi, dünya ve (x, y). Dünyaya basınca oraya warp atar. Vend'ler her dünyaya girişte otomatik `vends.json`'a kaydedilir |
| `/scan [blocks\|bg\|float] [isim]` | Growscan penceresi: dünyadaki bloklar, arka planlar ve yerdeki eşyalar, sayılarıyla. İsim yazılırsa sadece eşleşenler. Kısayollar: `/gs`, `/blocks`, `/items` |
| `/vf [eşya]` | `/vendfind` kısayolu |
| `/pvend <eşya>` | Bu dünyada o eşyayı en ucuza satan vend'in yanına yürür |
| `/pfind <eşya>` | Yerdeki en yakın o eşyanın yanına yürür |
| `/tpto <oyuncu>` | Oyuncunun yanına yürür (isim başı yeterli) |
| `/collect`, `/autocollect` | Toplama penceresi: oto toplama aç/kapa, menzil (1–10 kare), sadece bir eşya veya hepsi, gem atlama, aralık, mesaj. Yürümez, uzaktan toplama isteği yollar. `/collect now` bir kere toplar, `/ac` oto toplamayı açar/kapatır |
| `/door <kapı ID>` | Bu dünyaya o kapı ID'siyle yeniden girer |
| `/admin` | Dünyadaki kilitler, sahipleri ve erişimi olanlar |
| `/locked` | Kapılar, etiketleri ve açıp açamayacağın |
| `/vendcopy` | Bu dünyanın vend fiyatlarını panoya kopyalar |
| `/roulette` | Rulet spin geçmişi (kim, kaç, ne zaman) |
| `/drophistory`, `/track` | Kim ne attı, kim ne aldı; `/track` bunları canlı olarak sohbete yazar |
| `/hidelevel` | "You need to be Level ..." yazılarını gizler |
| `/minimal` | Proxy'nin sohbet mesajlarını gizler (uyarılar ekranın ortasında görünmeye devam eder) |
| `/rndm` | Rastgele bir dünyaya warp |
| `/dd <adet>` | Diamond Lock bırakır. DL yetmezse Blue Gem Lock'u DL'ye çevirip atar (`/dd 30`, DL yok, BGL var → 1 BGL bozulur, 30 DL atılır). 100'ün katları ve yüzlükler doğrudan BGL olarak gider (`/dd 200` → 2 BGL, `/dd 250` → 2 BGL + 50 DL) |
| `/dropall [stop]`, `/dropall1` | Envanterdeki her şeyi ya da her eşyadan birer tane bırakır. Önce listeyle onay penceresi açılır; 0,4 sn arayla tek tek bırakılır, `/dropall stop` durdurur. Fist ve Wrench kalır |
| `/logoff`, `/logout` | Giriş ekranına döner |
| `/re` | Son proxy komutunu tekrar çalıştırır |
| `/terminal [on\|off]` | Bir sonraki açılış terminal modunda mı (on) pencerede mi (off) olsun; ayarı `config.toml`'a yazar |
| `/log [text\|game\|variant\|redact] [on\|off]` | Paket loglamayı gösterir veya açıp kapatır |

## Web paneli (isteğe bağlı)

Proxy varsayılan olarak vend fiyatlarını (bir dünyaya girince hemen), spin sonuçlarını (oyuncu adı olmadan) ve kendi hata satırlarını (dakikada bir) HTTPS ile panele gönderir. Ayarlar → **Panel paylaşımı**'ndan kapatılabilir. GrowID, şifre, token, sohbet ve oyuncu isimleri gönderilmez. Veriler [gtproxy.xyz](https://gtproxy.xyz) paneline gider; Ayarlar'dan başka bir adres de yazılabilir.

## Ayarlar (`config.toml`)

| Bölüm | Alan | Varsayılan | Açıklama |
|---|---|---|---|
| `proxy` | `local_ip` | `127.0.0.1` | Oyunun bağlanacağı adres |
| | `http_port` | `443` | server_data.php HTTPS portu (oyun 443 kullanır) |
| | `enet_port` | `17091` | ENet proxy portu |
| `upstream` | `server_data_hosts` | growtopia1, growtopia2 | Sırayla denenecek alan adları |
| | `dns_server` | `1.1.1.1:53` | Gerçek IP'yi çözmek için kullanılan DNS sunucusu |
| | `verify_tls` | `true` | Gerçek sunucunun sertifikasını doğrula |
| `system` | `manage_hosts` | `true` | hosts dosyasını otomatik düzenle |
| | `terminal` | `false` | Çift tıklayınca pencere yerine terminal modunu aç |
| `report` | `enabled` | `true` | Web paneline paylaşım (kapatılabilir) |
| | `url` | boş | Panel adresi, `https://` ile; boş = projenin paneli |
| | `vends` / `spins` / `errors` | açık | Hangi veriler gönderilir |
| | `cert_dir` | `certs` | Sertifikaların tutulduğu klasör |
| `log` | `level` | `info` | tracing filtresi (örn. `gtproxy=debug`) |
| | `text` / `game_packet` / `variant` | açık / kapalı / açık | Hangi paket türleri loglanacak |
| | `skip_spam` | `true` | Hareket ve ping paketlerini loglama |
| | `redact_login` | `true` | Login paketlerindeki token, mac, rid gibi alanları maskele |
| `command` | `prefix` | `/` | Komut ön eki |
| `ui` | `embed_game` | `false` | Gömmeli mod: oyun proxy penceresinin içinde |

## Kod yapısı

```
src/
├── main.rs              CLI, başlatma ve kapanış
├── config.rs            config.toml
├── http.rs              HTTPS server_data.php köprüsü
├── dns.rs               hosts dosyasını atlayan DNS A sorgusu
├── report.rs            isteğe bağlı panel paylaşımı (vend, spin, hata; kuyruk + HTTPS gönderici)
├── embed.rs             gömmeli mod: Growtopia penceresini bizim pencereye alır (Win32)
├── hosts.rs             hosts dosyasına işaretli blok ekleme/kaldırma
├── cert.rs              yerel CA ve sunucu sertifikası
├── state.rs             HTTP, ENet ve pencere arasında paylaşılan durum ve özet
├── engine.rs            proxy'yi başlatma/durdurma, Growtopia'yı açma
├── gui.rs               masaüstü penceresi (egui)
├── ui_log.rs            pencerenin Log bölümü için log tamponu
├── items.rs             oyunun items.dat önbelleğinden eşya isimleri
├── protocol/            tel formatı: mesaj tipleri, GameUpdatePacket, VariantList, key|value metin
└── proxy/
    ├── mod.rs           ENet köprüsü (downstream ve upstream host'lar)
    ├── interceptor.rs   paket kararları: ilet / at / değiştir (soket kodu yok)
    ├── commands.rs      oyun içi komutlar ve proxy pencerelerinin buton işlemleri
    ├── roulette.rs      rulet spin tanıma, QQ/Reme hesabı
    ├── fast_item.rs     fast drop / fast trash (adet penceresini atlama)
    ├── tools.rs         hesap, envanter, dünya sahibi ve görünüm komutları
    ├── inventory.rs     envanter takibi
    ├── locks.rs         kilit hesabı (/cd, /balance)
    ├── features.rs      özelliklerin açık/kapalı durumu
    ├── settings.rs      settings.json, /autosave, /options
    ├── social.rs        oto pull/ban, ban words, isim/kıyafet gizleme, /lp
    ├── worlds.rs        kayıtlı dünyalar (/sworld, /list)
    ├── chat_tools.rs    /me, /formalize, /calc, /timer, ping uyarısı
    ├── stats.rs         rulet istatistiği
    ├── world.rs         dünya haritası okuyucu (tile'lar, kilit/giriş erişimi)
    ├── teleport.rs      Ctrl+tık ışınlanma: menzil genişletme, yol bulma, hareket
    ├── zoom.rs          sınırsız zoom (kendi OnSpawn'ımızda mstate 1) + /zoom <seviye>
    ├── fake_drop.rs     /dlocal: yerel sahte Dirt ve eşya numarası çevirisi
    ├── scan.rs          /scan: Growscan benzeri blok ve yerdeki eşya sayımı
    ├── vends.rs         vend kaydı (vends.json) ve /vendfind
    ├── info.rs          /admin, /locked, /vendcopy, /hidelevel, /minimal, /rndm, /re
    ├── history.rs       /roulette, /drophistory, /track
    ├── pathing.rs       /pvend, /pfind, /tpto, /door
    ├── collect.rs       /collect penceresi ve oto toplama
    ├── dropall.rs       /dropall, /dropall1, /logoff
    ├── dialog.rs        oyun içi pencere (OnDialogRequest) oluşturucu
    ├── effect.rs        proxy'nin ürettiği paketler ve gecikmeli işler
    ├── session.rs       oyuncu ve dünya takibi
    └── log.rs           paket loglama
```

Yeni bir paket kancası `interceptor.rs` içine, yeni bir komut ise `commands.rs` içindeki `COMMANDS` listesine eklenir.

## Test

```bash
cargo test
```

Birim testleri; paket, variant, metin, DNS ve hosts işlemlerinin yanında `OnSendToServer` yeniden yazımını ve komutları kapsar. Bir uçtan uca test de localhost'ta sahte bir istemci ve iki sahte sunucuyla gerçek köprüyü çalıştırır: ilk bağlantı, paket iletimi, komut yakalama, alt sunucu yönlendirmesi ve yeniden bağlanma.

## Not

Bu araç Growtopia'nın kullanım şartlarına aykırı olabilir. Hesabın banlanması ihtimali dahil sorumluluk kullanıcıya aittir.
