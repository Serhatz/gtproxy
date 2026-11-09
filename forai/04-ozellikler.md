# 04: Özellikler ve nerede oldukları

Tüm oyun içi komutlar `proxy/commands.rs` içindeki `COMMANDS` tablosundadır. Penceredeki butonlar da aynı komutları `Shared::command` ile çalıştırır. Proxy'nin tanımadığı komutlar (`/msg`, `/help`, `/pull`...) **sunucuya aynen gider**.

## Komut → dosya

| Komut(lar) | Ne yapar | Dosya |
|---|---|---|
| `proxyhelp`, `proxy`, `log` | komut penceresi, durum penceresi, paket logu | commands.rs |
| `warp`, `back`, `relog`, `exit` | dünya değiştirme. `/warp` proxy'nindir, sunucuya gitmez (oyunun `/warp`'ı cooldown'lu ve herkes kullanamıyor). `warp::go_to()`: `quit_to_exit`, `OnRequestWorldSelectMenu` gelince hemen `join_request`; 3 sn cooldown | commands.rs, warp.rs |
| `spin [csn\|qq\|reme\|toggle\|instant]`, `spinstats [reset]` | rulet kontrolü (`[REAL]`, QQ/Reme), istatistik | commands.rs, roulette.rs, stats.rs |
| `fastdrop`, `fasttrash` | adet penceresini atla | commands.rs, fast_item.rs, interceptor.rs |
| `dw`, `dd`, `db`, `daw`, `cd` | kilit bırakma; `/cd` gerekirse kilit bozar. `/dd n`: `locks::plan_dl_drop` (yeterli DL → DL; yoksa yüzlükler BGL, kalan DL; DL yetmezse BGL bozulur, 200 yığın sınırı gözetilir) | commands.rs, fast_item.rs, locks.rs, tools.rs |
| `balance`, `inventory`, `account`, `debug`, `news` | bilgi pencereleri | tools.rs |
| `change` | Telephone ile 100 DL → 1 BGL (Sales-Man akışı; **oyunda doğrulanmadı**) | tools.rs |
| `setsave`, `save`, `sworld`, `list` | kayıtlı dünyalar | tools.rs, worlds.rs |
| `wrench`, `pullall`, `kickall`, `banall`, `lp`, `chat` | dünya sahibi araçları (`/pull isim` sohbet komutuyla) | tools.rs, social.rs, commands.rs |
| `autopullpage`, `autobanpage`, `addpull`, `addban`, `banwords` | girene oto pull/ban, yasaklı kelimeye ban | social.rs |
| `hidename`, `hideclothes` | isim/kıyafet gizleme (sadece istemcide) | social.rs |
| `nick`, `skin`, `country`, `ui` | sadece istemcide görünen değişiklikler; boş `/nick` ve `/skin` geri alır | tools.rs |
| `spam` | iki metinli otomatik spam (en az 2 sn aralık) | tools.rs, features.rs |
| `me`, `formalize`, `calc`, `timer`, `pingalert` | sohbet araçları, sayaç, ping uyarısı | chat_tools.rs |
| `options`, `autosave` | ayar penceresi, `settings.json` | settings.rs |
| `tp [x y \| speed ms]` | Ctrl+tık ışınlanma | teleport.rs, world.rs |
| `zoom [seviye]` | sınırsız zoom (OnSpawn mstate 1, sonraki dünyada) | zoom.rs, tools.rs |
| `dlocal [adet\|clear]` | sadece sende görünen sahte **Dirt** | fake_drop.rs |
| `vendfind [eşya]` | dünyaya girince vend'leri `vends.json`'a kaydeder, arama penceresi, warp | vends.rs, world.rs |
| `scan [blocks\|bg\|float] [isim]`, `gs`, `blocks`, `items` | Growscan: blok, arka plan ve yerdeki eşya sayıları (haritadan, sunucuya bir şey gitmez) | scan.rs, world.rs |
| `vf` | `vendfind` kısayolu | vends.rs |
| `dropall [stop]`, `dropall1` | önce onay penceresi; sonra `tick` ile 0,4 sn arayla `action\|drop`, cevap penceresi `PlannedDrop` (`quiet`) ile otomatik cevaplanır. Fist/Wrench atlanır | dropall.rs, fast_item.rs |
| `logoff`, `logout` | `action\|quit` (oyunda **doğrulanmadı**) | dropall.rs |
| `pvend`, `pfind`, `tpto` | dünya içinde yürüme: `teleport::walk()` yolu tek seferde yürür | pathing.rs, teleport.rs |
| `collect [now]`, `autocollect`, `ac` | toplama penceresi + oto toplama (`tick`). Yürümeden `ItemActivateObjectRequest` (value = uid, vec = eşyanın konumu) yollar; menzil 1–10 (varsayılan 3), tek eşya filtresi, gem atlama, aralık; aynı uid 2 sn içinde tekrar istenmez; çanta doluysa istemez. Ayarlar kalıcı | collect.rs, settings.rs |
| `door <id>` | `WORLD\|ID` ile aynı dünyaya yeniden girer | pathing.rs, warp.rs |
| `admin`, `locked` | kilit sahipleri/erişim (`WorldMap::locks`), kapı etiketleri (`Special::Door`). İsimler `session.known_users` (OnSpawn userID) ve giriş mesajındaki "World Locked by" satırından | info.rs, world.rs, session.rs |
| `vendcopy` | vend fiyatlarını `clip.exe` ile panoya kopyalar | info.rs |
| `hidelevel`, `minimal` | "You need to be Level" baloncuk/konsol satırlarını düşürür; `say()` mesajlarını gizler (`warn` kalır). Kalıcı ayar | info.rs, settings.rs |
| `terminal [on\|off]` | `config.toml`'u okuyup `system.terminal`'ı değiştirir ve kaydeder (sonraki açılış). Açık pencerenin bellekteki ayarı güncellenmez; pencereden Kaydet'e basılırsa onunki yazılır | info.rs |
| `rndm`, `re` | rastgele 4–7 harfli dünyaya warp; son komutu tekrar (`features.last_command`) | info.rs, commands.rs |
| `roulette`, `drophistory`, `track` | spin geçmişi; drop/toplama geçmişi. Drop'u atanı sunucu söylemez: en yakın oyuncuya (3 kare) yazılır. Diğer oyuncuların konumu `session.player_pos` (OnSpawn posXY + sunucudan gelen State) | history.rs, interceptor.rs |

Arka planda çalışanlar (komutsuz):
- Rulet `[REAL]` etiketi ve anında gösterme.
- Login alanlarının logda maskelenmesi.
- Envanter ve dünyadaki oyuncuların takibi.
- Seviye ve gem bilgisinin Track paketlerinden okunması.
- Dünya haritası ve tile güncellemeleri.
- Gazette'nin `/news` için saklanması.

## Masaüstü penceresindeki yerleri

| Sayfa / kart | Komutlar |
|---|---|
| Özellikler → Toplama | `ac`, `collect range N`, `collect only <eşya\|all>`, `collect now`, `collect` |
| Özellikler → Diğer | `minimal`, `hidelevel` (ping, autosave yanında) |
| Dünya → Dünya değiştir | `rndm` ve kapı kutusu (`door ID`) |
| Dünya → Bu dünya | `gs`, `items`, `admin`, `locked`, `vendcopy`, `drophistory`, eşya kutusu (`vendfind`, `pvend`, `pfind`), `track` |
| Dünya → Oyuncular | her satırda Git (`tpto isim`) |
| Rulet | "Oyunda geçmiş" (`roulette`), "Kim attı" listesi (`Snapshot::spin_history`) |

Gömmeli modda (`ui.embed_game`) aynı sayfalar sol panelde tek sütun olarak çizilir; üst çubuk (`embed_bar`) ışınlanma, oto toplama, fast drop ve warp'ı her zaman gösterir.

Yeni bir durum gösterilecekse `state.rs::Snapshot` + `proxy/mod.rs::Bridge::snapshot` + `gui.rs` demo verisi (`demo_snapshot`) üçü birlikte güncellenir.

## Oyuncu jargonu sözlüğü

| Terim | Anlamı |
|---|---|
| WL / DL / BGL | World Lock (242) / Diamond Lock (1796, 100 WL) / Blue Gem Lock (7188, 100 DL) |
| CSN | düz rulet |
| QQ | spin sayısının birler basamağı (23 → 3) |
| Reme | rakamlar toplamının birler basamağı (23 → 5, 29 → 1, 19 → 0) |
| `[REAL]` | sunucunun gerçek spin mesajı, sohbetten sahte yazılmış değil |
| pull / kick / ban | dünya sahibinin oyuncuya uyguladığı işlemler, oyunun `/pull isim` komutlarıyla yapılır |
| fast drop | drop'ta "kaç tane?" penceresini atlayıp hepsini bırakmak |

## Yeni bir özellik ekleme tarifi

1. **Durum**: kalıcı bir ayar ise `features.rs` → `Features` alanı. Oturumluk bilgiyse `session.rs`.
2. **Mantık**: ilgili modüle bir fonksiyon `pub fn x(ic: &mut Interceptor, args: &[&str])`. Mesaj için `say(ic, ...)`, önemli uyarı için `warn(ic, ...)` kullan; ikisi de ilk harfi büyütür, `warn` ekranın ortasına yazar.
3. **Paket kancası** gerekiyorsa `interceptor.rs`:
   - istemciden gelen metin: `on_client_text`
   - istemciden gelen game paketi: `on_client_game`
   - sunucudan gelen game paketi: `on_server_game`
   - sunucudan gelen fonksiyon çağrısı: `on_call`
   - sunucudan gelen dialog: `on_server_dialog`

   Değiştirilen paket `Verdict::Replace(Message::Game(pkt).encode())` ile döner.
4. **Komut**: `commands.rs` → `COMMANDS` tablosuna bir satır.
5. **Oyun içi pencere** gerekiyorsa `Dialog::new("isim")` kullan. Butonların cevabı için `on_dialog` zincirine (commands.rs) kendi `on_dialog` fonksiyonunu ekle. Pencereyi `tools::show(ic, dialog, "Close", "")` ile aç; böylece `/ui` renklerini alır.
6. **Kalıcılık**: `settings.rs` → `Settings` yapısına alan ekle; `snapshot` ve `apply` fonksiyonlarını güncelle. Eski dosyalar okunabilsin diye `#[serde(default)]` var.
7. **Masaüstü penceresi**: `state.rs` → `Snapshot`'a alan ekle, `proxy/mod.rs` → `Bridge::snapshot` ile doldur, `gui.rs` ilgili sayfada göster. Butonlar `self.cmd("komut")` çağırır.
8. **Test**: paket seviyesinde test yaz (`feature_tests.rs` örnekleri). Testte `Interceptor::new(Arc::new(Config::default()))` kullan; Ctrl gibi sistem girdileri için `teleport.ctrl` gibi değiştirilebilir fonksiyon alanları var.
9. README'deki komut tablosuna ve gerekirse bu belgelere ekle.

## Bekleyen / konuşulmuş işler

- **AutoSurg**: önce cerrahi dialoglarının tam markup'ını ve basılan butonları kaydeden bir "kayıt modu" yazılmalı. Kullanıcı birkaç ameliyat yapacak, kurallar loglardan çıkarılacak. Tanınmayan durumda otomasyon durmalı.
- **Takas koruması**: takas penceresi paketleri hiç loglanmadı; önce kayıt.
- **`/change`**, **`/news`**, **`/sworld :kapı`** (WORLD|DOOR join biçimi), **`/hidename`** (boş isim ``` `` ```): oyunda doğrulanmadı.
- **Oyunda doğrulanmadı (2026-11-06'da eklendi)**: `/collect` ve oto toplama (sunucunun kabul ettiği mesafe; 3 güvenli sanılıyor), `/hidelevel` metni, `/admin` sahip satırı, `/door`, `/vendcopy`'nin panoya Türkçe karakterle yazması.
- Kullanıcının önerilen fikir listesi: spam filtresi, anti-lag (partikül gizleme), kopunca oto dönüş, `/ignore`, arkadaş bildirimi, CCTV/kilit geçmişi, `/vendcopy`, `/fvend`, `/admin`.
