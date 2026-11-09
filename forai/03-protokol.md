# 03: Protokol notları (gerçek oyundan doğrulanmış)

Buradaki bilgilerin çoğu **gerçek oyun loglarından** doğrulandı (oyun sürümü 5.61, protokol 227). "Doğrulanmadı" yazanlar referans projelerden alınmıştır. Yeni bir akış eklerken önce logla, sonra uygula.

## Giriş akışı

1. Oyun → `POST /growtopia/server_data.php` (`version=5.61&protocol=227&platform=0`).
   - Cevap `key|value` satırlarıdır: `server|IP`, `port|`, `loginurl|`, `meta|`, `type2|1`... ve sonda çıplak `RTENDMARKERBS1001` satırı.
   - **`#maint|...` satırı her zaman vardır ve yorumdur**; aktif bakım `maint|...` olur.
2. Oyun ana sunucuya bağlanır → `ServerHello` → login metni (`protocol|`, `ltoken|`...).
3. Sunucu `OnSendToServer` ile alt sunucuya yönlendirir → oyun yeniden bağlanır ve tam login metnini gönderir (`tankIDName|`, `klv|`, `rid|`, `mac|`, `token|`...).
4. `OnSuperMainStartAcceptLogonHrdxs47254722215a` → `action|enter_game` → dünya.

**Proxy tarafındaki önemli noktalar:**
- Proxy kendi gerçek IP sorgusunu hosts dosyasını atlamak için 1.1.1.1'e yapar.
- Gerçek sunucunun TLS sertifikası doğrulanır (`reqwest` + `resolve`).
- Oyun **proxy'nin sertifikasını doğrular**: CA kurulu değilse TLS `DecryptError` verir ve oyun "Check your internet" der.

## Mesaj tipleri (ENet yükünün ilk `u32`'si)

| Tip | Anlam |
|---|---|
| 1 | ServerHello |
| 2 | Generic text (login, `action|input`, `dialog_return`, `action|drop`...) |
| 3 | Game message (`action|join_request`, `action|quit_to_exit`) |
| 4 | Game packet (56 bayt + ekstra) |
| 6 | Track (analitik: `Level|`, `Gems_balance|`) |

### Metin paketlerinde son bayt (önemli)

Metnin **son baytı her zaman sonlandırıcı yeridir** ve oyun onu her zaman sıfırlamaz. Logda `0x0F`, `*`, `0xB4` görüldü. Bu bayt metne eklenirse komutlar tanınmaz (`/fastdrop*`). `message.rs` **son baytı her zaman atar**, sonra ilk NUL'da keser. GTProxy de aynısını yapıyor.

### Sohbet ve komutlar
- Sohbet: `action|input\n|text|merhaba`. Baştaki `|` dikkat: `text.rs` bunu tolere eder. Yeniden yazarken bu biçim korunmalıdır.
- Başka oyuncunun sohbeti istemciye `OnTalkBubble [1]=netID, [2]="CP:0_PL:0_OID:_player_chat=METİN"` olarak gelir.

### `/warp` (doğrulandı)
Oyun sadece `action|input
|text|/warp X` gönderir; gerisi sunucuda. Cevap: `action|log` ile komutun tekrarı (`` `6/warp x`` ``), `OnSetFreezeState 1`, `msg|Magically warping to world X...`, ardından dünyadan çıkmadan doğrudan yeni harita (toplam ≈0.27 sn). Bekleme süresinde: `` msg|`4Cooldown active`` - You can use `5/warp`` in `53 ``seconds. `` (dünya değişmez). Çık + gir: `quit_to_exit` → ≈0.16 sn sonra `OnRequestWorldSelectMenu` → `join_request` → ≈0.22 sn sonra harita.

## GameUpdatePacket (56 bayt)

`[0]` tip, `[1]` object_type, `[2]` jump_count, `[3]` animation_type, `net_id`, `target_net_id`, `flags`, `float_var`, `value`, `vec_x/y`, `vec_x2/y2`, `particle_rotation`, `int_x/y`, ekstra uzunluk. Ekstra veri `EXTENDED` (0x08) bayrağıyla gelir.

| Paket | Bilinen anlamı |
|---|---|
| `CallFunction` (1) | ekstra = VariantList. **`value` = gecikme (ms)**; rulet mesajı 1800 ms gelir. |
| `State` (0) | hareket. Tıklama: `PUNCH`/`PLACE` + `TILE_CHANGE` bayrakları, `int_x/y` = tıklanan kare. **Boş kareye tıklamada sadece State gider.** |
| `TileChangeRequest` (3) | istemci: bloğa yumruk (`value=18`) ya da koyma (`value=eşya`). Sunucu: kare değişti (18 = kırıldı). |
| `SetCharacterState` (0x14) | **`jump_count = 126 + build menzili`, `animation_type = 126 + punch menzili`.** Ring of Range ile 131/131 görüldü (menzil 5). |
| `SendMapData` (4) | dünya haritası (bkz. aşağı). |
| `SendInventoryState` (9) | `u8 ?, u32 slot, u16 adet, adet × (u16 id, u8 miktar, u8 bayrak)`. |
| `ModifyItemInventory` (13) | `value` = eşya, `jump_count` = azalan, `animation_type` = artan. |
| `ItemChangeObject` (14) | `net_id=-1` yeni yere düşen eşya (`value`=eşya, `float_var`=adet, `vec`=konum; uid'yi **istemci sırayla verir**); `net_id=oyuncu` toplandı (`value`=uid). |
| `ItemActivateRequest` (10) | eşyayı "kullan": kilit çevirme (100 WL→DL, DL→100 WL, BGL→100 DL), giyinme. `value`=eşya. |
| `ItemActivateObjectRequest` (11) | yerden eşya toplama, `value`=uid. |

## VariantList

`u8 adet`, sonra her biri `u8 index, u8 tip, değer`. Tipler: 1 float, 2 string (`u32 uzunluk`), 3 vec2, 4 vec3, 5 unsigned, 9 signed. Tanınmayan tip görülürse ayrıştırma durur ve paket olduğu gibi iletilir.

| Fonksiyon | Argümanlar |
|---|---|
| `OnSendToServer` | `[1] port, [2] token, [3] userId, [4] "ip|doorID|UUIDToken", [5] lmode, [6] isim`. Proxy [1] ve [4]'ün IP kısmını değiştirir; tipleri (Signed/Unsigned) korur. |
| `OnSpawn` | `[1]` metin: `netID|`, `userID|`, `name|` (renk kodlu), `country|tr|...`, yerel oyuncuda `type|local`. |
| `OnSetClothing` | `[1..3]` vec3 kıyafetler, `[4]` **ten rengi (unsigned)**, `[5]` vec3. |
| `OnDialogRequest` | `[1]` dialog markup'ı. |
| `OnConsoleMessage`, `OnTalkBubble`, `OnTextOverlay` | sohbet satırı, konuşma balonu, ekran ortası yazı. |
| `OnZoomCamera` | `[1]` float seviye (0.1 = en uzak), `[2]` int ms. Oyun her seviyeyi kabul eder; tekerlek sınırı istemcidedir. |
| `OnNameChanged`, `OnChangeSkin`, `OnCountryState`, `OnSetPos (vec2)` | sadece istemcide görünen değişiklikler. |

## Dialoglar

- Markup satırları: `add_label_with_icon|big|`wBaşlık``|left|ID|`, `add_textbox|metin|left|`, `add_text_input|id|etiket|değer|max|`, `add_button|id|etiket|noflags|0|0|`, `add_checkbox|id|etiket|0/1|`, `embed_data|key|value`, `end_dialog|isim|İptal|Tamam|`.
- Cevap: `action|dialog_return\ndialog_name|isim\n<alanlar>\nbuttonClicked|id`. **Close ya da X için cevap gelmez.**
- Proxy dialog isimleri `gtproxy_` ile başlar; bu cevaplar sunucuya **gitmez**.

### Drop ve trash (doğrulandı)
- Oyun `action|drop\n|itemID|ID` gönderir. Sunucu `end_dialog|drop_item|...` gönderir; sayı kutusu (`add_text_input|count||ADET|5|`) **elindeki adetle dolu**, `embed_data|itemID|ID` var.
- Trash/recycle (`trash_item`): sayı kutusu **0** ile gelir; sahip olunan adet metindedir: `How many to `4destroy``? (you have 3)`.
- Cevap: `action|dialog_return\ndialog_name|drop_item\nitemID|ID|\ncount|N`.
- Elinde olmayan eşya için sunucu **hiç cevap vermez**. Bu yüzden proxy önce kendi envanterine bakar.

### Rulet (doğrulandı)
`OnTalkBubble` ve `OnConsoleMessage` ikilisi, `net_id=-1`, delay 1800: `` `7[```2İsim`` spun the wheel and got `423``!`7]`` ``. Sayıdan önceki karakter renk kodudur: `4` kırmızı, `b` siyah, `2` yeşil.

### Oyuncular, kilit sahibi, level yazısı
- Başka oyuncunun `OnSpawn`'ı: `netID`, `userID`, `posXY|x|y` (piksel) içerir. Hareketleri sunucudan `State` paketi (net_id = onun netID'si) olarak gelir. `/tpto` ve drop sahipliği bunlardan beslenir.
- Dünyaya girişte konsol: `` `5[```wDÜNYA`` `$World Locked`` by İSİM (`2ACCESS GRANTED``)`5]`` `` (doğrulandı). Sahip adı buradan okunur.
- "You need to be Level ..." metni diğer proxy'nin dökümünden alındı, **oyunda doğrulanmadı**. `/hidelevel` bunu içeren baloncuk ve konsol satırlarını düşürür.

### Yerdeki eşyayı almak
- İstemci → sunucu `ItemActivateObjectRequest` (tip 11): `value` = obje uid'si, `vec_x/vec_y` = objenin konumu. GrowBots ve diğer proxy de yürümeden, birkaç kare uzaktan bunu gönderiyor (diğer proxy'nin üst sınırı 10 kare, GrowBots 1–5). Sunucunun gerçek sınırı **doğrulanmadı**; varsayılan 3.
- Biri aldığında sunucu `ItemChangeObject` gönderir: `net_id` = alan oyuncu, `value` = uid. Yeni drop'ta `net_id = -1` ve **atanın kim olduğu yazmaz**.

## Dünya haritası (`SendMapData`)

`u16 sürüm, u32 bayrak, u16 isim_uzunluğu, isim, u32 genişlik, u32 yükseklik, u32 kare_sayısı, 5 bayt`. Sonra her kare için: `u16 fg, u16 bg, u16 parent, u16 bayrak, [HAS_PARENT ise u16], [HAS_EXTRA ise u8 tür + ekstra], [CBOR listesindeki eşyalarda u32+CBOR]`. Karelerden sonra 12 bayt, `u32 obje_sayısı, u32 son_uid`.

- Ekstra türlerinin boyutları GrowBots'tan uyarlandı (`world.rs::parse_extra`). **Bilinmeyen tür → ayrıştırma hata verir** ve ham veri `last_world.dat` olarak kaydedilir. Tahmin yürütmek yasak, çünkü bir kare bile kaysa haritanın geri kalanı bozulur.
- Gerçek 5.61 haritası (100×60) hatasız okundu.
- Kapı ekstrası (tür 1): `string etiket, u8`. **Kapının ID'si ve gittiği yer oyunculara gönderilmez**; sadece etiket var (`Special::Door`). `/door ID` bu yüzden `join_request name|DÜNYA|ID` ile yeniden girer.
- Kilit ekstrası (tür 3): sahip userID'si ve erişim listesi (negatif değerler müzik BPM'idir). `WorldMap::locks()` hepsini verir; isimler oturumda görülen `OnSpawn` userID'lerinden eşlenir.

## Oyun dosyaları

- `%LOCALAPPDATA%\Growtopia\cache\items.dat` (**sürüm 27**). Genel ayrıştırıcılar 24–25'e kadar. `items.rs` her kaydın başını arar: `u32 id, u16, u8, u8, u16 isim_uzunluğu, XOR'lu isim` (anahtar `PBG892FXX982ABC*`, başlangıç `id % 16`) ve ardından `.rttex` ile biten doku adı. **Çarpışma türü, doku adından sonraki 14. bayttır** (0 boş, 1 katı, 2 platform...).
- Önemli eşya ID'leri: World Lock 242, Diamond Lock 1796, Blue Gem Lock 7188, Roulette Wheel 758, Telephone 3898, Ring of Range 11094, Dirt 2.
- `.rttex`: `RTPACK` (32 bayt) + zlib. İçinde `RTTXTR` başlığı (124 bayt): **önce yükseklik, sonra genişlik**, sonra format, orijinal yükseklik, orijinal genişlik. Ardından alttan üste RGBA satırlar, ikinin kuvvetine kadar dolgulu.
- Font: `GameData\UI\Font\Century-Gothic-Bold.ttf` (oyunun kendi fontu). Çalışırken oradan yüklenir, projeye kopyalanmaz.
- Açılış arka planı: `game\hills1/2/3.rttex`, `cloud.rttex`, `sun.rttex`.
