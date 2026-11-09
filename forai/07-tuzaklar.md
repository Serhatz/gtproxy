# 07: Tuzaklar (bizi gerçekten yakanlar)

Her madde bu projede yaşanmış bir hatadır. Benzer bir şeye dokunmadan önce oku.

## Ağ ve protokol

1. **rusty_enet, `using_new_packet` ile hiç bağlanmamış host'ta panik yapar.** Başlık bütünlük baytları peer slot 0'ın adresinden hesaplanıyor; slot hiç kullanılmadıysa `unwrap` ile çöküyor. Çözüm: upstream host'u ilk bağlantıdan önce `service` ya da `flush` etme (`last_target` kontrolü), upstream'i **tek slot** (`peer_limit 1`) ile kur.
2. **Upstream'i nazikçe kapatmak slotu meşgul bırakır.** Oyun alt sunucuya geçerken birkaç ms içinde yeniden bağlanıyor; `disconnect` yerine **`disconnect_now`** kullan.
3. **Windows UDP 10054 (`ConnectionReset`)**: oyunun eski portu kapanınca soket bu hatayı verir ama zararsızdır, debug seviyesinde yut.
4. **Metin paketinin son baytı** çöp olabilir (`0x0F`, `*`): her zaman at. Atılmazsa komutlar "bazen" tanınmaz.
5. **Trash/recycle penceresi sayı kutusunu 0 ile gönderir**; adet "(you have N)" metninden okunur.
6. **Elinde olmayan eşya için sunucu sessiz kalır.** Bu yüzden önce envantere bak; planlı istekler için zaman aşımı kullan (3 sn), yoksa sonraki elle yapılan drop'u yakalarsın.
7. **İstemci yerdeki eşyalara uid'yi kendisi sırayla verir.** İstemciye sahte obje eklemek uid'leri kaydırır; iki yönlü çeviri şarttır (`fake_drop.rs`).
8. **SetCharacterState dünyaya girişte OnSpawn'dan önce gelebilir**: netID henüz bilinmezken de kabul et.
9. **Konum çevirisi**: `tile = floor((x + 10) / 32), floor((y + 15) / 32)`. Gerçek bir logla (1093,738 → 34,23) doğrulandı.
10. **Işınlanma sırasında** oyunun kendi `State` paketleri sunucuya gitmemeli (`busy_until`), yoksa karakter geri çekilir. Sonunda istemciye `OnSetPos` gönderilir.

11b. **Drop'u kimin attığı pakette yok.** `ItemChangeObject` (net_id −1) sadece eşya, adet ve konum taşır; `history.rs` en yakın oyuncuya (3 kare) yazar. Kesin bilgi gibi gösterme.
11c. **Toplama isteğini aynı uid için tekrarlama.** Oto toplama her geçişte aynı objeyi istemesin diye uid 2 sn bekletilir; çanta doluyken de istenmez (`collect.rs`).

## Oyun dosyaları

11. **items.dat sürüm 27**: alan alan ayrıştırma kırılır. Kayıt başı taramasıyla isim ve çarpışma türü okunur.
12. **`.rttex` başlığında yükseklik genişlikten önce gelir**, satırlar alttan üste, boyutlar ikinin kuvvetine dolgulu. Orijinal alanı kırp.
13. **Harita ekstrasında bilinmeyen tür** → ayrıştırmayı durdur, `last_world.dat`'a yaz. Tahmin edersen haritanın geri kalanı kayar.
14. Dünya menüsüne çıkarken 0×0 boyutlu boş bir harita gelebilir; bu bir hata değildir.

## Windows ve dağıtım

15. **Çalışma klasörü**: `target\release` içinden çift tıklanınca program orada yeni sertifika üretiyordu; oyun bu kurulu olmayan CA'yı reddetti. `use_project_dir()` önce `Cargo.toml`'un olduğu atayı, sonra `config.toml`'un olduğunu seçer. Bunu bozma.
16. **Konsol penceresi X ile kapatılınca** eskiden süreç öldürülüyor ve hosts kirli kalıyordu. `tokio::signal::windows::ctrl_close` ve arkadaşları dinleniyor; pencerede `Drop` motoru durduruyor.
17. **hosts yazmak yönetici ister.** Pencere `PermissionDenied` görünce kendini `ShellExecuteW("runas", ..., "--start")` ile yeniden başlatır.
18. **Ctrl tuşu** paketlerden anlaşılmaz; `GetAsyncKeyState(VK_CONTROL)` ile okunur. Proxy ile oyun aynı makinede olmalı.
19. **Sınırsız zoom**: tekerleği hook ile taklit etmek (OnZoomCamera göndermek) oyunun kendi zoom'uyla çekişir, takılır ve en uzakta bir anlık normale zıplar. Doğru yol: kendi `OnSpawn`'ımızda (`type|local`) `mstate|1` yapmak; istemci moderatör gibi sınırsız zoom'a izin verir, sunucuya bir şey gitmez (enetproxy'den). OnSpawn dünyaya girişte geldiği için değişiklik sonraki dünyada geçerli olur. Yerel OnSpawn'ı yeniden göndermek karakteri çoğaltır, yapma.
19b. **Ctrl+tık menzili**: genişletilmiş menzil istemciye sadece Ctrl basılıyken gösterilir (`teleport::tick` her döngüde Ctrl'i okur). Sürekli gösterilirse Ctrl'siz de uzağa vuruluyormuş gibi görünür.

## egui 0.37

20. **API değişti**:
    - `App::update` yerine `fn ui(&mut self, ui: &mut egui::Ui, frame)`.
    - Paneller `egui::Panel::top/left/bottom(id).show(ui, ...)` ve `CentralPanel::default().show(ui, ...)` biçiminde.
    - `Frame::new()`, `.corner_radius(CornerRadius::same(u8))`, `Margin::same(i8)`.
    - Stil için `ctx.global_style_mut`.
    - Font için `ctx.add_font(FontInsert::new(...))`.
21. **Varsayılan fontta olmayan semboller** (●, ▶, ■, →, ↔) uyarı verip görünmez. Şekil gerekiyorsa `painter` ile çiz ya da ASCII kullan.
22. Saydamlık için `Color32::gamma_multiply` kullan (premultiplied renklerde doğru sonucu verir).

22b. **Taşan kartlar**: `ui.columns` sütun genişliği verir ama içerik ondan genişse kart taşar ve komşusunun üstüne biner. Kurallar: sütunlar `flow()` ile (pencere genişliğine göre 4/2/1), büyük yazılar `shadowed()` ile sığmazsa küçülür, metin + sağda buton/switch satırları `labeled_row()` (sağ taraf önce yerleşir, yazı kırılır), kutular sabit genişlik yerine `available_width()` kullanır.

22c. **Kutu + birkaç buton aynı satırda** dar pencerede taşar (`field_row` etiketi 110 px yer kaplar). Yazı kutusunu kendi satırına koy (`desired_width(available_width())`), butonları `horizontal_wrapped` ile alt satıra al. Toplama ve Bu dünya kartlarında böyle düzeltildi.

22d. **Gömmeli mod: oyunu çocuk pencere yapma (`SetParent`).** Oyun DPI'dan habersiz ve pencere boyutunu dinlemiyor (`SetWindowPos`, `WM_SIZE`, `WM_ENTERSIZEMOVE/EXITSIZEMOVE` denendi: küçülen pencerede aynı kare çizilir, sol alt köşesi görünür). Çocukken Windows ölçeklemesi bozuldu ve görüntü, ne boyut verilirse verilsin, üç denemede de kırpıldı. Doğrusu: oyunu **üst pencere olarak bırak**, başlık çubuğunu kaldır, istemci boyutunu koru, bizim pencereye sahiplendir (`GWLP_HWNDPARENT`) ve sadece taşı. Böylece her zamanki gibi çizilir, klavye de normal çalışır. Boyutu oyunun kendi çözünürlük ayarı belirler (%125'te 1280x800 → ekranda 1600x1000).
22e. **Sahip pencere kapanınca sahiplenilen pencereler de yok edilir.** `embed.release()` kapatma isteğinde, `App::on_exit`'te ve `Drop`'ta çağrılır; yeni bir çıkış yolu eklersen orada da bırak.
22g. **Gömülü oyunun üstüne kendi penceremizde çizilemez.** Sayfa paneli ayrı, kenarlıksız bir egui viewport'udur ve **oyun penceresine** sahiplendirilir (`own_window`), böylece oyuna tıklansa da üstünde kalır. Önizlemede (`GTPROXY_EMBED=panel`) panel `egui::Area` olarak çizilir, çünkü ekran görüntüsü sadece ana pencereyi alır.

## Ekran görüntüsü otomasyonu (PowerShell)

Artık `GTPROXY_SHOT` var (bkz. 06); aşağıdakiler sadece dışarıdan yakalama gerekirse geçerli.


23. `Add-Type` ile tanımlanan sınıf aynı oturumda tekrar tanımlanamaz; eski tanım kalır ve yeni metotlar "yok" olur. **Her betikte yeni bir sınıf adı** kullan.
24. `FindWindow` güvenilmez; `Start-Process -PassThru` ile `MainWindowHandle` dolana kadar bekle.
25. DPI ölçeklemesi yüzünden `GetWindowRect` ile `CopyFromScreen` uyuşmaz; `SetProcessDPIAware` çağır. Yine de görüntü kısmen kırpık olabilir; bu, arayüzün bozuk olduğu anlamına gelmez.

## Süreç

26. Python ile toplu düzenleme yaparken heredoc içinde `$` ya da tırnak karmaşası bash'i bozabilir; uzun yamaları scratchpad'e `.py` dosyası olarak yazıp çalıştır. Her değişiklikten önce `assert s.count(a) == 1` ile tam bir eşleşme olduğunu doğrula.
27. Dosyayı Edit ile değiştirmeden önce Read ile okumuş olmak gerekir. Başka bir araç dosyayı değiştirdiyse tekrar oku.
