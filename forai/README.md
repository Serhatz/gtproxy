# forai: AI ajanları için başlangıç noktası

Bu klasör, `yeniproje` (gtproxy) üzerinde çalışacak bir AI ajanının **koda dokunmadan önce** okuması gereken her şeyi içerir. Kod kendini anlatır; burada koddan çıkarılamayan bilgiler var: neden böyle yapıldığı, gerçek oyunda öğrenilen protokol gerçekleri, kurallar ve tuzaklar.

## 60 saniyede proje

- **Ne:** Growtopia için Rust ile yazılmış bir **MITM proxy** ve onun **masaüstü uygulaması**. Oyun istemcisi proxy'ye bağlanır, proxy gerçek sunucuya bağlanır; aradaki paketler okunur, değiştirilir ve proxy'nin kendi komutları eklenir.
- **Nasıl açılır:** `target\release\gtproxy.exe` → pencere → **Başlat**. Proxy hosts dosyasını yönlendirir ve Growtopia'yı açar. Yönetici izni gerekirse uygulama kendini yönetici olarak yeniden başlatır.
- **Dil:** Kullanıcıyla Türkçe konuşulur. Pencere arayüzü Türkçedir; oyun içi mesajlar ve kod yorumları İngilizcedir.
- **Durum:** Çalışıyor ve gerçek oyunla test edildi. 54 oyun içi komut var; 91 birim/akış testi geçiyor.

## Okuma sırası

| Dosya | İçerik |
|---|---|
| [01-genel-bakis.md](01-genel-bakis.md) | Projenin amacı, kapsamı, kapsam dışı olanlar, kullanıcı tercihleri |
| [02-mimari.md](02-mimari.md) | Thread'ler, veri akışı, **dosya dosya proje haritası** |
| [03-protokol.md](03-protokol.md) | Gerçek oyun loglarından öğrenilen paket formatları ve akışlar |
| [04-ozellikler.md](04-ozellikler.md) | Tüm komutlar ve hangi dosyada oldukları; yeni özellik ekleme tarifi |
| [05-kurallar.md](05-kurallar.md) | **Mutlaka uyulması gereken kurallar** (commit yok, yapılmayacak özellikler, kod stili) |
| [06-calisma-akisi.md](06-calisma-akisi.md) | Derleme, test, önizleme, ekran görüntüsü, gerçek oyunda test |
| [07-tuzaklar.md](07-tuzaklar.md) | Bu projede bizi yakan hatalar ve nedenleri |

## En önemli 6 kural (ayrıntısı 05-kurallar.md'de)

1. **Asla `git commit` yapma ve önerme.** Kullanıcı açıkça istemedi.
2. **Dupe, exploit, moderatörden saklanma, casino dolandırıcılığı ve sahte kanıt** özellikleri yapılmaz.
3. Sunucuya, gerçek oyunun asla göndermeyeceği bir paket gönderme. Emin olmadığın akışı önce logla, sonra uygula.
4. Gerçek oyunda doğrulanmamış bir şeye "çalışıyor" deme. Testten geçti demek oyunda çalıştı demek değildir.
5. hosts dosyası ve sertifika kullanıcının sistemidir. Proxy sadece kendi işaretli bloğunu ekler/siler; sistem ayarına sen dokunma.
6. Her değişiklikten sonra `cargo test` ve `cargo build --release` çalıştır.
