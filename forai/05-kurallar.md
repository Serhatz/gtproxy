# 05: Kurallar

## Kullanıcının koyduğu kurallar

1. **Asla `git commit` yapma, commit önerme, commit butonu gösterme.** Değişiklikler commit edilmeden kalır.
2. **Kullanıcıyla Türkçe konuş.** Pencere arayüzü Türkçe; oyun içi mesajlar (sohbet/dialog) İngilizce.
3. **Uyarılar ekranın ortasında** (`warn` → `OnTextOverlay`). Bilgi mesajları sohbette (`say`).
4. **Mesajlar büyük harfle başlar.** `say`/`warn` bunu `sentence()` ile otomatik yapar. Renk koduyla başlayan metne (oyuncu isimleri) dokunulmaz.
5. **Görünüm Growtopia'ya benzemeli**: oyunun fontu, oyundan ölçülmüş renkler, gölgeli yazılar, alt kenarı koyu butonlar, yarı saydam paneller. Renk eklerken tahmin etme; kullanıcının ekran görüntüsünden ölç.
6. GrowBots (`../src`, `../web`) ve `../proxy` **sadece referanstır**. Fikir alınır, kod kopyalanmaz ve bu klasörler değiştirilmez.

## Yapılmayacaklar (pazarlık konusu değil)

- **Eşya kopyalama / dupe**, sunucuda eşya var etme, "düşür ama envanterden silme" denemeleri.
- Sunucuya, gerçek oyunun asla göndermeyeceği **bozuk/sahte paketler** göndermek (yanlış adet, olmayan eşya, tekrar edilen istekler).
- **Moderatörden saklanma**: mod tespiti, mod gelince çıkma, "legit mode".
- **Casino dolandırıcılığı araçları**: vergisiz ödül bırakma, hosting/split araçları. Rulet `[REAL]` kontrolü serbesttir, çünkü dolandırıcılığa karşı korur.
- **Başkalarını kandıracak sahte kanıt**: başkalarına görünen sahte drop, sahte değerli eşya. `/dlocal` sadece istemcide görünen **Dirt** düşürür; bilerek BGL değildir.

Kullanıcı böyle bir şey isterse nedenini kısaca söyle ve güvenli bir alternatif öner.

## Güvenlik ve sistem

- **hosts dosyası**: sadece `# >>> gtproxy` / `# <<< gtproxy` arasındaki blok eklenir/silinir. Dosyanın geri kalanına asla dokunulmaz; ilk düzenlemede yedek alınır. Proxy hangi yoldan kapanırsa kapansın blok temizlenmeli (HostsGuard, pencere Drop'u, konsol kapatma sinyalleri).
- **Sertifika**: sadece **mevcut kullanıcının** deposuna, kullanıcı butona basınca/komut verince kurulur (`certutil -user`). Ajan kendiliğinden kurmaz, kaldırmaz.
- **Yönetici izni ve sistem ayarları**: ajan hosts dosyasını değiştiren, sertifika kuran ya da yönetici olarak çalışan bir komutu **kendisi çalıştırmaz**. Kullanıcıya adımları verir.
- Login verisi (token, mac, rid, klv...) logda maskelenir (`proxy/log.rs::redact`, `/log redact on`). Bunları loglarda ya da cevaplarında açığa çıkarma.

## Kod kuralları

- Çevredeki kodun stiline uy: kısa doc yorumları, İngilizce yorumlar, anlamlı isimler, gereksiz yorum yok.
- `Interceptor` saf mantıktır; soket ya da thread işi ekleme. Yan etkiyi `Effect` ile ifade et.
- Paket ayrıştırmada `unwrap` yerine `Option`/`?` kullan. Ayrıştırılamayan paket **olduğu gibi iletilir** (`Verdict::Forward`), asla düşürülmez.
- Değiştirdiğin paketin tiplerini koru: Signed ise Signed, Unsigned ise Unsigned.
- Sunucuya giden yeni bir akışı önce **gerçek logla doğrula**. Doğrulanmamışsa koda ve kullanıcıya bunu söyle.
- Her yeni davranış için test yaz; mümkünse gerçek oyundan alınmış örnek veriyle (`fast_item.rs::parses_real_recycle_dialog` gibi).
- Pencere metinleri Türkçe; fontta olmayan sembol kullanma (●, ▶, →, ↔, ■ gibi; bkz. 07).
- Yeni bağımlılık eklerken Windows'ta ek araç (cmake, nasm) gerektirmeyenleri seç (ör. rustls için `ring`, `aws-lc` değil).

## Kullanıcıya rapor verirken

- Ne yapıldığını, ne test edildiğini ve **neyin oyunda denenmediğini** açıkça söyle.
- Log okurken iddiaları log satırlarına dayandır.
- Kullanıcı log okutmak için "Denedim, logu oku" der; log `yeniproje/gtproxy.log` dosyasıdır.

## Panel paylaşımı (report.rs, server/)

- `server/` (gtproxy.xyz sitesi ve paneli), `strings.txt` ve `KOMUTLAR.md` **yayınlanmaz**; `.gitignore`'da. Yeni özel dosya eklersen oraya da ekle.

- Varsayılan **açık** (proje sahibinin kararı); Ayarlar'dan kapatılabilir. Ne gönderildiği pencerede ve README'de yazılı kalmalı.
- Gönderilmez: GrowID, şifre, token, MAC/RID, sohbet, **oyuncu isimleri**, IP (sunucu sadece tuzlu hash tutar). Yeni bir veri türü eklerken bu listeyi koru, `scrub()`'dan geçir.
- Sunucu herkese açık: her alan doğrulanır, sınırlıdır; kurulum ve IP başına hız sınırı; panelden engelleme ve silme.
