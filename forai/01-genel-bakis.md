# 01: Genel bakış

## Proje nedir

gtproxy, Growtopia oyun istemcisi ile resmi sunucu arasına giren bir proxy'dir:

```
Growtopia.exe ──HTTPS──> gtproxy :443   ──HTTPS──> www.growtopia1.com  (server_data.php)
Growtopia.exe ──ENet───> gtproxy :17091 ──ENet───> gerçek sunucu / alt sunucular
```

Proxy:
- hosts dosyasıyla oyunun alan adlarını kendine yönlendirir,
- `server_data.php` cevabındaki sunucu adresini kendi adresiyle değiştirir,
- ENet trafiğini iki yönde iletir, okur ve gerektiğinde değiştirir,
- sohbete yazılan `/komut`ları yakalayıp kendi özelliklerini çalıştırır,
- oyun içi pencereler (dialog) ve bir masaüstü penceresi (egui) sunar.


## Kapsam

**Kapsamda:** proxy çekirdeği, oyun içi komutlar ve dialoglar, masaüstü penceresi (egui), ayar kaydetme, dünya haritası/ışınlanma, rulet kontrolü, envanter/kilit araçları, sosyal araçlar (pull/ban listeleri vb.).

**Bilerek kapsam dışı:**
- Lua scripting ve web paneli. Kullanıcı "şu an alanımız değil" dedi.
- Moderatör tespiti/kaçışı (`/moddetect`, `/legitmode`, "mod gelince dur").
- Casino hosting araçları (`/nw`, `/qq` spin, `/csn`, `/xsplit`). Rulet **kontrolü** (`[REAL]`) ise kapsamda.
- Eşya kopyalama/dupe, sunucuya sahte drop, başkalarını kandıracak sahte kanıt (ör. herkese görünen sahte BGL drop).
- AutoSurg/Startopia otomasyonu. Konuşuldu ama başlanmadı; önce kayıt modu gerekir (bkz. 04).

## Kullanıcı hakkında

- Projenin sahibi; Türkçe konuşur, oyunu iyi bilir, gerçek oyunda test eder ve log dosyasını sana okutur.
- Özellik isteklerini genelde oyuncu jargonuyla anlatır ("fast drop", "dw", "reme", "csn"). Emin değilsen sor; 04-ozellikler.md'de bu terimlerin karşılıkları var.
- Arayüzün Growtopia'ya benzemesini istiyor: oyunun fontu, renkleri, butonları ve arka planı.
- Uyarıların sohbete değil **ekranın ortasına** düşmesini, mesajların **büyük harfle** başlamasını istiyor.

## Teknoloji

- Rust 2024 edition; araç zinciri 1.96 (`~/.cargo/bin/cargo`).
- ENet: `rusty_enet` (CLOEI fork, git bağımlılığı). Hem `using_new_packet` (istemci) hem `using_new_packet_server` (sunucu) destekler.
- HTTPS: `tokio` + `tokio-rustls` (ring) + `hyper` (sunucu) ve `reqwest` (gerçek sunucuya istemci).
- Sertifika: `rcgen` ile yerel CA + sunucu sertifikası (`certs/`).
- Pencere: `eframe`/`egui` 0.37 (glow backend). API'si eski egui örneklerinden farklıdır (bkz. 07).
- Diğer: `serde`, `toml`, `serde_json`, `flate2` (oyun dokuları), `tracing`.
