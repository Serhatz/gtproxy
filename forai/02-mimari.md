# 02: Mimari ve proje haritası

## Çalışma zamanı: kim nerede çalışır

```
main thread ──────── egui penceresi (gui.rs)
   │                    │  Engine::start / stop
   │                    ▼
   │              "proxy" thread (engine.rs)  ── tokio runtime
   │                    ├── http::serve           HTTPS :443, server_data.php köprüsü
   │                    └── spawn_blocking ──► proxy::run  ENet köprüsü (1 thread, 1 ms döngü)
   │                                              └── Bridge { downstream, upstream, Interceptor }
   ├── "zoom-hook" thread (zoom.rs)   Windows fare tekerleği dinleyicisi (isteğe bağlı)
   └── Shared (state.rs)  ◄── hepsinin ortak noktası (Arc)
```

- **`Shared`**: server_data'dan gelen gerçek sunucu adresi, durdurma bayrağı, TLS hata bayrağı, pencereden gelen **komut kuyruğu** ve pencere için **Snapshot**.
- **Pencere ↔ proxy iletişimi** iki yönlüdür:
  - Pencere → proxy: `shared.command("tp speed 40")`. ENet döngüsü kuyruğu boşaltır ve `commands::run` ile çalıştırır. Yani penceredeki her buton bir oyun içi komuttur.
  - Proxy → pencere: ENet döngüsü 250 ms'de bir `Bridge::snapshot()` ile `Shared::publish` çağırır, pencere bunu okur.
- **Komut satırı modu**: `gtproxy run` pencere açmadan aynı `Engine`'i çalıştırır. Ctrl+C ya da konsol kapatma geldiğinde hosts temizlenir.

## Bir paketin yolculuğu

```
oyun ──ENet──► downstream host ──► Interceptor::handle(ToServer, bytes) ──► Verdict
                                                                   ├─ Forward  → upstream'e aynen
                                                                   ├─ Replace  → değiştirilmiş bayt
                                                                   └─ Drop     → gönderilmez
                     + Interceptor'ın ürettiği Effect'ler (ToClient / ToServer / Later)
sunucu ──ENet──► upstream host ──► Interceptor::handle(ToClient, bytes) ──► ... ──► oyun
```

- `Interceptor` (proxy/interceptor.rs) **soket bilmez**; sadece bayt alır, karar verir ve `Effect` üretir. Bu sayede testler soket açmadan paket akışını doğrular (`feature_tests.rs`).
- `Effect::Later(delay, e)` gecikmeli iştir; `Scheduler` (effect.rs) zamanı gelince uygular.
- `Interceptor::tick()` her döngüde çağrılır: spam, timer, ping uyarısı, autosave, süresi dolan planlı drop'lar ve zoom.

## Bağlantı yaşam döngüsü (proxy/mod.rs)

1. Oyun `server_data.php` ister. `http.rs` gerçek IP'yi DNS'ten (1.1.1.1) alır, isteği gerçek sunucuya iletir ve cevaptaki `server|`/`port|` alanlarını proxy adresiyle değiştirir. Gerçek adres `Shared`'a yazılır.
2. Oyun ENet ile proxy'ye bağlanır → `on_game_connect`. Hedef şu sırayla seçilir: `redirect` (son OnSendToServer) → server_data adresi → son hedef.
3. Sunucu `OnSendToServer` gönderince (alt sunucuya geçiş) Interceptor IP/portu proxy adresiyle değiştirir ve gerçek hedefi `redirect`'e saklar. Oyun kopup yeniden bağlanır; proxy saklanan hedefe gider.
4. Upstream host'un **tek** peer slotu var (bkz. 07). Oyun koptuğunda upstream `disconnect_now` ile hemen kapatılır.

## Dosya haritası (`src/`)

### Kök
| Dosya | Görev |
|---|---|
| `main.rs` | Giriş noktası. Argümansız → pencere; `run` → konsol modu; `cert install/uninstall`, `hosts remove`, `help`. `use_project_dir()` çalışma klasörünü proje köküne çeker. Loglama: konsol, `gtproxy.log` ve pencerenin Log sayfası. |
| `engine.rs` | `Engine::start/stop`: sertifika kontrolü, hosts kurulumu (yönetici izni yoksa `StartError::NeedAdmin`), proxy thread'i. `launch_game`, `game_path`. |
| `gui.rs` | egui penceresi: tema, sayfalar (Genel bakış, Özellikler [Toplama kartı], Dünya [Bu dünya kartı: Growscan, kilitler, kapılar, vend ara/git, drop geçmişi], Rulet [kim attı], Log, Ayarlar), oyun stili bileşenler (`card`, `gt_button`, `switch`, `shadowed`), yönetici olarak yeniden başlatma. Geliştirme sürümünde önizleme modu. |
| `gui_background.rs` | Oyunun `.rttex` dokularını çözer; "Açılış" ve "Bağlantı" arka plan sahnelerini çizer. |
| `report.rs` | Panel paylaşımı (`[report]`, varsayılan açık, `url` boşsa `DEFAULT_REPORT_URL`). `vends()` / `spin()` statik kuyruğa yazar (kapalıyken hemen döner), `ErrorLayer` tracing WARN/ERROR satırlarını `scrub()` ile temizleyip ekler. `report` thread'i vendleri dünyaya girince (`NUDGE`, 2 sn toplama), spin/hataları dakikada bir JSON olarak `{url}/api/ingest.php`'ye POST eder; başarısızsa sınırlı tekrar. Kurulum kimliği `report_id.txt` (rastgele 32 hex). Sunucu: `server/` (PHP + MySQL, cPanel; `server/README.md`). **`server/` özeldir, GitHub'a gitmez** (`.gitignore`); depoyu klonlayan birinde bu klasör olmaz. |
| `terminal.rs` | Terminal modu (`gtproxy run` ya da `system.terminal = true`): proxy'yi arka planda çalıştırır, terminalde sadece log görünür (komut almaz; kullanıcı böyle istedi). Ctrl+C / pencereyi kapatma durdurur ve hosts'u geri alır. Çift tıklamada konsolu `AllocConsole` açar; yönetici gerekirse `run` argümanıyla yeniden başlar. |
| `embed.rs` | Gömmeli mod. `Growtopia.exe`'nin görünür üst penceresini `EnumWindows` + süreç adıyla bulur, başlık çubuğunu kaldırır, `GWLP_HWNDPARENT` ile bizim pencereye **sahiplendirir** (çocuk değil) ve boş alanın ortasına taşır; boyutuna dokunmaz. `release()` eski stil, sahip ve konumu geri yükler. GUI tarafı `gui.rs::embedded_ui`: üst çubuk `embed_bar`, siyah alan, açılır sayfa paneli `side_overlay` (varsayılan kapalı; ayrı egui viewport'u, oyun penceresine sahiplendirilir ki hep üstünde dursun). |
| `ui_log.rs` | Son 3000 log satırını bellekte tutar (Log sayfası için). |
| `config.rs` | `config.toml`: proxy, upstream, system, log, command ve `ui` (görünüm) bölümleri. Yoksa varsayılanla oluşturur. |
| `state.rs` | `Shared` ve `Snapshot`. |
| `http.rs` | HTTPS sunucusu, server_data köprüsü, TLS hatasında `note_tls_failure`. |
| `dns.rs` | Elle yazılmış DNS A sorgusu; hosts dosyasını atlamak için. |
| `hosts.rs` | hosts dosyasına **işaretli blok** ekler/siler; yedek alır. `HostsGuard` drop'ta temizler. |
| `cert.rs` | Yerel CA ve sunucu sertifikası üretimi; `certutil -user` ile kurma/kaldırma. |
| `items.rs` | Oyunun `items.dat` önbelleğinden eşya **isimleri** ve **çarpışma türleri** (sürümden bağımsız tarayıcı). |

### `protocol/`: tel formatı (oyundan bağımsız, saf veri)
| Dosya | Görev |
|---|---|
| `message.rs` | `u32` mesaj tipi + gövde. Metin paketlerinde **son bayt her zaman atılır** (bkz. 03). |
| `game_packet.rs` | 56 baytlık `GameUpdatePacket`, tip enum'u, bayraklar. |
| `variant.rs` | `VariantList` (fonksiyon çağrıları: `OnConsoleMessage`, `OnSendToServer`...). |
| `text.rs` | `key|value` satırları; satır sırası korunur, çıplak satırlar (`RTENDMARKERBS1001`) korunur. |

### `proxy/`: köprü ve tüm özellikler
| Dosya | Görev |
|---|---|
| `mod.rs` | `Bridge`: iki ENet host, bağlantı yaşam döngüsü, komut kuyruğu, Snapshot. Uçtan uca ENet testi burada. |
| `interceptor.rs` | **Merkez**: her paketi görür, `Verdict` döner. Oturum, envanter, dünya, ışınlanma, spin, drop dialogları buradan dağıtılır. |
| `commands.rs` | `COMMANDS` tablosu, `run`, `say`, `warn`, `sentence`; durum, rulet ve komut dialogları; kilit drop komutları. |
| `dialog.rs` | Oyun içi dialog oluşturucu (`Dialog`) ve dialog cevabı ayrıştırıcı (`DialogReturn`). İsim öneki `gtproxy_`. |
| `effect.rs` | `Effect` (ToClient/ToServer/Later) ve hazır paketler: console, overlay, dialog, chat, client_call, activate_item. |
| `session.rs` | Oyuncu/dünya bilgisi: isim, netID, userID, konum, dünyadaki oyuncular, seviye, gem, ten rengi. |
| `log.rs` | Paket loglama ve login alanlarının maskelenmesi. |
| `features.rs` | Açık/kapalı durumlar ve listeler (`Features`, `Spam`, `NameList`, `SavedWorld`). |
| `settings.rs` | `settings.json` (oyun içi ayarlar), `/autosave`, `/options`. |
| `fast_item.rs` | Fast drop/trash ve planlı drop'lar (`/dw`...). |
| `locks.rs` | Kilit hesabı ve `/cd` bozma planı. |
| `inventory.rs` | Envanter takibi. |
| `tools.rs` | Hesap, envanter, kilit, görünüm (`/nick`, `/country`...), dünya sahibi araçları, spam, `/change`. |
| `social.rs` | Oto pull/ban listeleri, yasaklı kelimeler, isim/kıyafet gizleme, `/lp`. |
| `worlds.rs` | Kayıtlı dünyalar (`/sworld`, `/list`). |
| `chat_tools.rs` | `/me`, `/formalize`, `/calc`, `/timer`, ping uyarısı. |
| `stats.rs` | Rulet istatistiği. |
| `roulette.rs` | Spin tanıma, `[REAL]` etiketi, QQ/Reme hesabı. |
| `world.rs` | Dünya haritası okuyucu (`SendMapData`), tile güncellemeleri, erişim kontrolü. |
| `teleport.rs` | Ctrl+tık ışınlanma: menzil genişletme, BFS yol bulma, hareket paketleri. |
| `zoom.rs` | Sınırsız tekerlek zoom (Windows low-level mouse hook + `OnZoomCamera`). |
| `fake_drop.rs` | `/dlocal`: sadece istemcide görünen sahte Dirt ve eşya uid çevirisi. |
| `feature_tests.rs` | Paket seviyesinde özellik testleri. |

## Çalışma zamanında oluşan dosyalar (proje kökünde)

| Dosya | Ne | Git |
|---|---|---|
| `config.toml` | Uygulama ayarları (+ `[ui]` görünüm) | yok sayılır |
| `settings.json` | Oyun içi özellik ayarları (`/autosave`) | yok sayılır |
| `certs/` | CA ve sunucu sertifikası; **özel anahtarlar burada** | yok sayılır |
| `gtproxy.log` | Her açılışta sıfırlanan log | yok sayılır |
| `last_world.dat` | Okunamayan son harita (hata ayıklama için) | yok sayılır |

Program `target\release\` içinden çalıştırılsa bile `use_project_dir()` sayesinde bu dosyalar proje kökünde aranır.
