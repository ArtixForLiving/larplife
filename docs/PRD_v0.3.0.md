# Product Requirements Document (PRD): `larplife` (Revisi Final v0.3.0)

**Versi:** v0.3.0 (Tahap Pengembangan Awal — Full Rust)  
**Target Sistem Operasi:** Windows 10 / 11 (64-bit)  
**Bahasa Pemrograman:** 100% Rust (Edition 2024)  
**Target Kinerja (NFR):** $< 10\text{ MB}$ RAM di Latar Belakang (Aktual: $2.1\text{--}5.8\text{ MB}$), Beban CPU $\sim 0\%$, Ukuran Binary $< 10\text{ MB}$

---

## 1. Latar Belakang & Tujuan Produk

Saat pengguna membuka aplikasi kerja, kreatif, atau aplikasi buatan sendiri di PC (seperti Photoshop, CapCut, Blender, maupun software kustom seperti `KambingKuring.exe`), fitur pendeteksi bawaan Discord sering kali tidak menampilkan logo aplikasi, tidak memiliki keterangan aktivitas yang jelas, atau bahkan tidak mendeteksi aplikasi tersebut sama sekali. Di sisi lain, membuat Application ID secara manual satu per satu di Discord Developer Portal sangat merepotkan bagi pengguna awam.

**Tujuan `larplife` v0.3.0:**
Menyediakan aplikasi latar belakang yang $100\%$ mandiri (*standalone*), ringan, dan ditulis sepenuhnya dengan bahasa **Rust** untuk:
1. Membaca aplikasi yang sedang dibuka pengguna secara instan ($< 1\text{ detik}$) dan menampilkannya ke Discord Rich Presence lengkap dengan **logo aplikasi besar (`large_image`)**, **badge bulat identitas `larplife` di pojok kanan bawah (`small_image`)**, nama dokumen aktif yang sudah dibersihkan, serta durasi waktu kerja.
2. Menghilangkan kerumitan pembuatan Application ID manual melalui **Sistem 3 Lapis Application ID** (pencarian otomatis ke database publik Discord ber-cache lokal dengan pengaman anti-salah deteksi, mode bawaan `larplife`, serta tombol bantuan cepat).
3. Memudahkan pemasangan logo aplikasi melalui dukungan link gambar HTTPS, **ekstraksi icon langsung dari file `.exe`**, dan **unggah gambar lokal otomatis berlapis (*multi-host*)** dari jendela pengaturan tanpa membuat UI membeku.

---

## 2. Arsitektur Modul Program (Full Rust)

Untuk menjaga pemakaian RAM latar belakang tetap di angka $2\text{--}6\text{ MB}$, program memisahkan tugas antara **Proses Latar Belakang (Background Daemon)** dan **Proses Jendela Pengaturan (`--settings`)**. Seluruh koneksi internet (pencarian API Discord & upload gambar) hanya dijalankan di dalam proses pengaturan pada *worker thread* terpisah dan langsung dimatikan saat jendela ditutup.

| No | Nama File Modul | Posisi Proses | Tugas & Tanggung Jawab Utama |
| :--- | :--- | :--- | :--- |
| 1 | `src/main.rs` | Background & CLI | Pengaman agar aplikasi tidak terbuka ganda (*Single Instance Mutex*), pengatur argumen `--settings`, serta logika utama pelacak status (*PresenceTracker*). |
| 2 | `src/worker.rs` | Background | Menjalankan pengecekan instan detik ke-0 saat startup dan loop pengecekan rutin menggunakan `mpsc::Receiver::recv_timeout`. |
| 3 | `src/config.rs` | Background & GUI | Membaca, memvalidasi (termasuk format URL `https://` dan batas panjang karakter), serta menyimpan `config.json` v0.3.0 dengan kompatibilitas mundur. |
| 4 | `src/window_reader.rs` | Background | Membaca nama `.exe` dan judul jendela aktif via Win32 API secara aman (tahan aplikasi Administrator, mendukung aplikasi UWP, dan bebas kebocoran memori handle). |
| 5 | `src/discord_client.rs` | Background | Mengelola koneksi Named Pipe (`0` s/d `9`) ke Discord (Stable, PTB, Canary), mengganti ID aplikasi secara dinamis, memasang badge `larplife`, dan memotong teks sesuai batas UTF-8. |
| 6 | `src/tray.rs` | Background | Menampilkan ikon di System Tray Windows, menangani menu klik kanan instan, serta memulihkan ikon otomatis jika Windows Explorer *restart* (`TaskbarCreated`). |
| 7 | `src/logger.rs` | Background & GUI | Mencatat riwayat error ke file `larplife.log` dan menampilkan kotak dialog Windows (`MessageBoxW`) jika pengaturan rusak. |
| 8 | `src/startup.rs` | Background & GUI | Mengatur pendaftaran aplikasi agar otomatis menyala saat Windows masuk ke desktop (Registry `HKCU\...\Run`). |
| 9 | `src/process_list.rs` | Khusus `--settings` | Mendata seluruh aplikasi yang sedang berjalan di desktop Windows (`EnumWindows` + `Toolhelp32`) untuk dipilih lewat dropdown. |
| 10 | `src/app_resolver.rs` | Khusus `--settings` | **(Baru di v0.3.0)** Mengelola cache disk `detectable_cache.json` (TTL $24\text{ jam}$) dan mencocokkan Application ID secara terukur (normalisasi nama + filter anti-salah deteksi). |
| 11 | `src/image_uploader.rs` | Khusus `--settings` | **(Baru di v0.3.0)** Mengekstrak icon dari resource `.exe` lokal atau membaca file gambar lokal, lalu mengunggahnya عبر *trait* `ImageHost` (Catbox.moe + host cadangan) untuk mendapatkan URL HTTPS. |
| 12 | `src/settings_gui.rs` | Khusus `--settings` | Jendela pengaturan berbasis `eframe`/`egui` dengan eksekusi jaringan *non-blocking* (via thread + channel), konfirmasi kandidat ID, unggah logo lokal, dan sinyal *reload* instan via `PostMessageW`. |

---

## 3. Spesifikasi Teknis Fitur Baru (Fokus v0.3.0)

### 3.1. Deteksi Instan Detik ke-0 & *Zero-Debounce* saat Status Kosong
Untuk menghilangkan jeda ~4 detik saat aplikasi pertama kali dibuka:
1. **Deteksi Detik ke-0 (*Instant Startup Check*):** Saat `worker.rs` dijalankan, program langsung mengeksekusi pemeriksaan jendela aktif pertama kali sebelum masuk ke antrean tunggu `recv_timeout` ($2\text{ detik}$).
2. **Kirim Langsung dari Status Kosong (*Zero-Debounce on Empty State*):**
   * Jika status aktif di `PresenceTracker` saat ini sedang **kosong (`None`)**, begitu pengguna membuka aplikasi terdaftar, status langsung dikirim ke Discord pada **siklus ke-1** ($< 1\text{ detik}$).
   * Aturan **Jeda Pergantian (*Switch Debounce* 2 siklus / $\sim 4\text{ detik}$)** hanya diaktifkan apabila pengguna berpindah secara langsung dari **Aplikasi Terdaftar A** $\to$ **Aplikasi Terdaftar B** guna mencegah *rate-limit* saat pengguna menekan *Alt-Tab* cepat bolak-balik.
   * Jika pengguna kembali ke Aplikasi A sebelum *Grace Period* ($30\text{ detik}$) habis, status langsung dilanjutkan pada siklus ke-1 tanpa reset timer.

### 3.2. Desain Visual Kartu RPC (Logo Aplikasi Besar + Badge Bulat `larplife`)
Setiap status yang dikirim oleh `discord_client.rs` menggunakan struktur aset gambar dua lapis standar Discord Rich Presence:
1. **Gambar Utama (`large_image` — Kotak Besar):**
   * Menampilkan logo dari aplikasi yang sedang dibuka (misalnya logo Photoshop, Blender, atau logo kustom `KambingKuring`).
   * Mendukung nilai berupa **URL gambar HTTPS langsung** (`https://...` maksimal $256\text{ karakter}$, format `.png`, `.jpg`, `.webp`) maupun **Asset Key** alfanumerik dari Discord Developer Portal.
   * **Fallback Otomatis:** Jika `logo_key` dikosongkan atau berisi URL yang tidak valid (bukan `https://` atau mengandung spasi), program tidak akan crash melainkan otomatis memakai URL logo utama `larplife` (`default_logo_url`) dan mencatat peringatan ke `larplife.log`.
2. **Badge Pojok Kanan Bawah (`small_image` — Lingkaran Kecil):**
   * Secara bawaan menampilkan **logo bulat `larplife`** di pojok kanan bawah gambar utama saat `badge_enabled = true` (jika `false`, field `small_image` dan `small_text` tidak dikirim sama sekali).
   * Menggunakan URL HTTPS global pada field `badge_image_url` di `config.json`.
   * Teks tooltip saat kursor diarahkan ke badge kecil (`badge_text`) secara default dibentuk otomatis menggunakan makro versi paket `concat!("larplife v", env!("CARGO_PKG_VERSION"))` kecuali diubah manual oleh pengguna di `config.json`.

### 3.3. Sistem 3 Lapis Application ID, Cache Disk, & Skor Pencocokan Terukur

#### Lapis 1: Pencarian Otomatis Berbasis Cache Disk (`detectable_cache.json`)
Agar tidak mengunduh file JSON berukuran beberapa MB setiap kali membuka pengaturan:
1. **Manajemen Cache Disk:**
   * Saat `larplife.exe --settings` dibuka, thread latar belakang di GUI memeriksa file `%LOCALAPPDATA%\larplife\detectable_cache.json`.
   * Jika file belum ada atau usianya melewati **TTL $24\text{ jam}$**, program mengunduh daftar dari `https://discord.com/api/v9/applications/detectable` di background thread, menyaring hanya 3 field penting (`id`, `name`, `executables`), lalu menyimpannya ke disk.
   * **Stale-While-Error:** Jika unduhan pembaruan gagal karena tidak ada internet, program tetap memakai file cache lama yang ada di disk tanpa menampilkan error yang mengganggu. Tersedia pula tombol kecil `[Perbarui Database]` di GUI untuk memaksa unduh ulang.
2. **Daftar Hitam Nama File Pasaran (*Generic Exe Blacklist*):**
   * Pencarian otomatis wajib menolak pencocokan jika nama file `.exe` termasuk dalam konstanta daftar hitam (case-insensitive):
     `app.exe`, `main.exe`, `client.exe`, `launcher.exe`, `game.exe`, `start.exe`, `setup.exe`, `install.exe`, `uninstall.exe`, `unins000.exe`, `update.exe`, `updater.exe`, `crashreporter.exe`, `python.exe`, `pythonw.exe`, `javaw.exe`, `java.exe`, `node.exe`, `electron.exe`, `cmd.exe`, `powershell.exe`, `conhost.exe`.
3. **Normalisasi & Urutan Skor Pencocokan (`app_resolver.rs`):**
   * Sebelum dibandingkan, nama proses dan path dinormalisasi: diubah ke huruf kecil, diseragamkan garis miringnya (`\` $\to$ `/`), serta dibuang akhiran arsitektur umum (`_x64`, `x64`, `64`, `_win64`, `-win64`) pada nama dasar file.
   * Urutan prioritas skor pencocokan (hanya mengambil **1 kandidat dengan skor tertinggi**):
     * **Skor 100 (Sangat Akurat):** Path folder induk + nama file `.exe` cocok persis dengan entri `executables` di database Discord (contoh: `blender/blender.exe`).
     * **Skor 80 (Akurat):** Nama file `.exe` cocok persis (setelah normalisasi) DAN nama folder induk mengandung kata kunci dari nama aplikasi di Discord.
     * **Skor 60 (Cukup):** Nama file `.exe` unik (panjang nama dasar $\ge 5\text{ karakter}$, tidak ada di blacklist) cocok persis dengan nama `.exe` di database Discord.
4. **Konfirmasi 1-Klik oleh Pengguna (Dilarang *Auto-Lock* Diam-diam):**
   * Jika ditemukan kandidat terbaik, GUI menampilkan kotak konfirmasi tepat di bawah kolom App ID:
     > *"Ditemukan di database Discord: **[Nama Aplikasi di Discord]** (ID: `123456...`)"*  
     > Tombol: **`[Gunakan ID Ini]`** | **`[Bukan Aplikasi Ini (Pakai Mode larplife)]`**

#### Lapis 2: Mode Identitas Bawaan `larplife` (Fallback Tanpa Batas Kuota)
Jika aplikasi tidak ditemukan di database Discord, pengguna memilih *"[Bukan Aplikasi Ini]"*, atau pengguna mengosongkan kolom `discord_app_id`:
* Sistem otomatis memakai nilai `default_discord_app_id` dari `config.json` (yang memiliki nilai konstanta bawaan `DEFAULT_LARPLIFE_APP_ID`). Dengan disimpannya nilai ini di `config.json`, pengguna dapat menggantinya kapan saja tanpa menunggu update aplikasi jika ID bawaan mengalami kendala.
* Pada GUI ditampilkan keterangan singkat: *"Mode larplife aktif: Header atas akan tampil sebagai 'Playing larplife' dengan logo dan nama asli aplikasi Anda di bagian detail."*
* Baris `details` di Discord otomatis menyertakan `app_name` di bagian depan (contoh: `"KambingKuring — Menu Utama"`).

#### Lapis 3: Tombol Pintas "Buat ID Sendiri (15 Detik)"
* Tersedia tombol **`[Buat ID Sendiri di Browser]`** yang menyalin `app_name` ke *clipboard* Windows dan membuka `https://discord.com/developers/applications` di browser default pengguna (memakai kuota akun Discord pengguna sendiri, yang saat dokumen ini ditulis memiliki batas sekitar 25 aplikasi per akun).

### 3.4. Upload Gambar Lokal Berlapis (*Multi-Host*), Ekstraksi Icon `.exe`, & Transparansi Privasi

1. **Arsitektur Uploader Berlapis (`ImageHost` Trait):**
   * Untuk mencegah kegagalan tunggal jika satu layanan diblokir ISP atau sedang *down*, `image_uploader.rs` mendefinisikan trait `ImageHost` dengan urutan otomatis:
     * **Host Utama (Prioritas 1):** **Catbox.moe** (`https://catbox.moe/user/api.php`, `reqtype=fileupload`, penyimpanan permanen).
     * **Host Cadangan (Prioritas 2 / Fallback Otomatis):** **0x0.st** atau **uguu.se** (otomatis dicoba jika Host Utama mengalami *timeout* atau mengembalikan HTTP error).
   * Seluruh fungsi upload menerima trait abstraksi HTTP agar dapat diuji dengan *mock response* tanpa koneksi internet saat menjalankan `cargo test`.
2. **Ekstraksi Icon `.exe` yang Aman di Windows (`image_uploader.rs`):**
   * Saat pengguna menekan tombol **`[Ambil Icon dari .exe]`**, program mengekstrak icon resolusi terbaik (target $256\times256\text{ px}$ atau minimal $64\times64\text{ px}$) melalui dua tahap:
     * **Tahap A (Prioritas Utama):** Membaca langsung *byte* PNG mentah dari struktur grup resource icon (`RT_GROUP_ICON` / `RT_ICON`) di dalam file PE `.exe` jika tersedia (menghindari distorsi warna/alpha channel).
     * **Tahap B (Fallback Win32):** Menggunakan `PrivateExtractIconsW` / `GetDIBits` dengan konversi kanal warna eksplisit `BGRA` $\to$ `RGBA` agar latar belakang transparan tidak berubah menjadi kotak hitam, lalu meng-encode hasilnya ke buffer PNG di memori.
   * Jika file `.exe` tidak memiliki icon sama sekali atau terproteksi, GUI menampilkan pesan yang ramah dan mengarahkan pengguna ke tombol **`[Pilih File Gambar Lokal...]`** (mendukung `.png`, `.jpg`, `.jpeg`, `.webp`, `.ico` hingga maksimal $2\text{ MB}$).
3. **Transparansi Privasi Unggahan Gambar:**
   * Di bawah tombol upload pada GUI, ditampilkan catatan permanen yang jelas:
     > *"Catatan Privasi: Gambar yang diunggah akan disimpan di hosting publik pihak ketiga agar dapat ditampilkan oleh Discord. Gunakan URL manual jika Anda tidak ingin mengunggah file."*

---

## 4. Penanganan Error Jaringan & Concurrency di GUI (`--settings`)

Seluruh operasi jaringan (`app_resolver` dan `image_uploader`) **dilarang berjalan di thread utama rendering `egui`**. Operasi dijalankan menggunakan `std::thread::spawn` dengan komunikasi status melalui `std::sync::mpsc::channel`.

| Kondisi Kegagalan / Error | Perilaku Sistem & Tampilan di GUI |
| :--- | :--- |
| **Tidak ada koneksi internet / DNS gagal** | Indikator loading berhenti, tampilkan pesan kuning *"Tidak ada koneksi internet. Menggunakan mode lokal/larplife."*, kolom input tidak dirusak, tombol tetap bisa dicoba ulang. |
| **Request Timeout (Batas $10\text{ detik}$)** | Jika upload ke Host Utama timeout, otomatis coba Host Cadangan. Jika keduanya timeout, tampilkan *"Server upload tidak merespons, silakan coba lagi atau tempel URL manual."* |
| **API `detectable` gagal diunduh & tidak ada cache** | Lewati Lapis 1 secara senyap, catat ke `larplife.log`, dan langsung gunakan Mode Identitas Bawaan `larplife` (Lapis 2) tanpa memunculkan popup error yang menakutkan. |
| **Ukuran gambar $> 2\text{ MB}$ atau format salah** | Tolak sebelum diunggah, tampilkan pesan *"Ukuran gambar maksimal 2 MB (Format: PNG/JPG/WebP/ICO)."*, kolom `logo_key` tidak diubah. |
| **Respons host upload bukan URL `https://` valid** | Tolak hasil respons, catat isi respons ke `larplife.log`, dan jangan menimpa kolom `logo_key`. |
| **Ekstraksi icon `.exe` gagal / tidak ada icon** | Tampilkan pesan *"File .exe ini tidak memiliki icon yang dapat diambil. Silakan pilih file gambar manual."* |

---

## 5. Daftar Kebutuhan Fitur & Kriteria Penerimaan (Acceptance Criteria)

| Kode | Nama Fitur | Deskripsi Kebutuhan | Kriteria Penerimaan Terukur (Acceptance Criteria) |
| :--- | :--- | :--- | :--- |
| **FR-01 s/d FR-07** | **Fondasi Inti v0.2.x** | Deteksi Win32/UWP aman, Multi-pipe Discord `0`–`9`, sanitasi path & UTF-8 $120\text{ byte}$, Grace Period $30\text{ detik}$, System Tray + `TaskbarCreated`, Log + Startup, dan GUI terpisah. | Seluruh **37 unit test** bawaan pada `config`, `window_reader`, `discord_client`, `main`, `worker`, `tray`, `logger`, `startup`, `process_list`, dan `settings_gui` tetap lulus $100\%$. |
| **FR-08** | **Deteksi Instan Detik ke-0** | Pemeriksaan langsung saat worker menyala dan pengiriman siklus ke-1 saat status sebelumnya kosong. | Dari kondisi status kosong (`None`), status RPC terkirim dalam $< 1\text{ detik}$ sejak aplikasi aktif. Perpindahan langsung Aplikasi A $\to$ B tetap menunggu $2\text{ siklus}$ ($\sim 4\text{ detik}$). |
| **FR-09** | **Badge Identitas `larplife`** | Menampilkan logo bulat `larplife` di pojok kanan bawah (`small_image`) beserta teks versi otomatis. | Saat `badge_enabled = true`, payload RPC memuat `small_image` dan `small_text`. Saat `false`, kedua field tersebut bernilai `None` (tidak dikirim). |
| **FR-10** | **Fallback App ID Otomatis** | Mengizinkan `discord_app_id` dikosongkan dan otomatis beralih ke `default_discord_app_id`. | Aplikasi dengan `discord_app_id = ""` lolos validasi `config.rs`, terkoneksi memakai `default_discord_app_id`, dan teks `details` diawali dengan `app_name`. |
| **FR-11** | **Pencari App ID + Cache & Filter** | Mencari App ID ke database Discord (cache disk $24\text{ jam}$) dengan blacklist `.exe` pasaran dan konfirmasi 1-klik. | Semua nama di daftar blacklist menghasilkan `None`; file cache tersimpan di `%LOCALAPPDATA%`; kandidat tidak pernah mengubah `discord_app_id` sebelum tombol konfirmasi diklik. |
| **FR-12** | **Upload Gambar Berlapis & Icon `.exe`** | Mengekstrak icon dari `.exe` atau memilih gambar lokal, lalu mengunggahnya via `ImageHost` (Catbox + cadangan). | File $> 2\text{ MB}$ ditolak; jika host utama gagal otomatis berpindah ke host kedua; hasil akhir selalu berawalan `https://`; UI tidak *freeze* selama proses upload. |
| **FR-13** | **Pintas Buat ID Sendiri** | Menyalin nama aplikasi ke *clipboard* dan membuka Discord Developer Portal lewat 1 tombol di GUI. | Klik tombol menyalin string `app_name` secara akurat ke clipboard Windows dan memanggil pembuka URL browser default. |
| **NFR-01** | **Isolasi Memori & Keamanan** | Menjaga efisiensi daemon dan keamanan konfigurasi. | RAM daemon $< 10\text{ MB}$ dan CPU $\approx 0\%$ setelah `--settings` ditutup; semua URL eksternal wajib `https://`; tidak ada kredensial/token rahasia di dalam binary. |

---

## 6. Asumsi, Ketergantungan Eksternal, & Batasan Cakupan (Out of Scope)

### 6.1. Matriks Ketergantungan Eksternal & Mitigasi
| Ketergantungan Eksternal | Kegunaan | Risiko Jika Gagal / Bermasalah | Rencana Mitigasi di `larplife` v0.3.0 |
| :--- | :--- | :--- | :--- |
| **Catbox.moe** | Host utama unggah logo lokal | Diblokir ISP atau server sedang *down* | Otomatis *failover* ke Host Cadangan (`0x0.st`/`uguu.se`) atau input URL manual (GitHub raw/Imgur). |
| **API `applications/detectable` Discord** | Sumber pencarian App ID (Lapis 1) | Offline, format berubah, atau lambat | Disimpan di cache disk lokal (`TTL 24 jam`), *stale-while-error*, dan otomatis lanjut ke Lapis 2 jika gagal. |
| **App ID Bawaan `larplife`** | Identitas fallback (Lapis 2) | ID terhapus atau dibatasi di masa depan | Nilai disimpan di field `default_discord_app_id` pada `config.json` sehingga dapat diganti pengguna tanpa build ulang. |
| **GitHub Raw (Aset Badge/Logo Default)** | Menampilkan logo badge `larplife` | Gagal dimuat oleh proxy Discord | URL disimpan di `badge_image_url` pada `config.json` dan fitur badge dapat dimatikan (`badge_enabled: false`). |

### 6.2. Di Luar Cakupan Versi Ini (*Out of Scope v0.3.0*)
Agar pengembangan tetap fokus dan ukuran program tetap kecil, hal-hal berikut **tidak** termasuk dalam cakupan v0.3.0:
1. Dukungan sistem operasi selain Windows 10/11 64-bit (macOS dan Linux belum didukung).
2. Pembuatan Application ID otomatis menggunakan *User Token* atau server relay pihak ketiga (dihindari demi keamanan akun dan kemandirian aplikasi).
3. Tombol interaktif kustom (*Custom RPC Buttons*) dan fitur *Party/Join Game* pada kartu Discord.
4. Pemantauan multi-akun Discord secara bersamaan dalam satu sesi Windows.

---

## 7. Skema File Pengaturan (`config.json` v0.3.0)

Seluruh field baru wajib menggunakan atribut `#[serde(default)]` di `src/config.rs` agar file `config.json` dari versi sebelumnya tetap dapat dibaca tanpa error:

```json
{
  "check_interval_seconds": 2,
  "clear_timeout_seconds": 30,
  "default_discord_app_id": "1350000000000000000",
  "badge_enabled": true,
  "badge_image_url": "https://raw.githubusercontent.com/larplife-project/larplife/main/assets/larplife_badge.png",
  "badge_text": "larplife v0.3.0",
  "apps": [
    {
      "app_name": "Adobe Photoshop",
      "process_name": "Photoshop.exe",
      "discord_app_id": "1182390192837482910",
      "logo_key": "https://raw.githubusercontent.com/larplife-project/larplife/main/assets/photoshop.png",
      "logo_text": "Adobe Photoshop",
      "default_details": "Sedang mendesain",
      "state_text": "Workspace Aktif",
      "hide_window_title": false,
      "keep_active_in_background": false,
      "strip_title_words": [
        " - Adobe Photoshop",
        "@ 100%",
        "(RGB/8)",
        "(CMYK/8)"
      ]
    },
    {
      "app_name": "KambingKuring",
      "process_name": "KambingKuring.exe",
      "discord_app_id": "",
      "logo_key": "https://files.catbox.moe/contoh_logo.png",
      "logo_text": "KambingKuring App",
      "default_details": "Menggunakan KambingKuring",
      "state_text": "Aktif",
      "hide_window_title": false,
      "keep_active_in_background": false,
      "strip_title_words": [
        " - KambingKuring"
      ]
    }
  ]
}
```
> **Catatan Rilis Penting:** Nilai `default_discord_app_id` dan `badge_image_url` di atas adalah contoh struktur. Sebelum membuat tag rilis final `v0.3.0`, pastikan konstanta default di `src/config.rs` telah diisi dengan Application ID resmi `larplife` dan URL gambar badge yang aktif di repositori rilis.

---

## 8. Strategi Pengujian Otomatis (Unit Test Plan v0.3.0)

Selain mempertahankan 37 unit test yang sudah ada, pengembangan v0.3.0 wajib menyertakan pengujian otomatis tambahan berikut (menggunakan *mock trait* tanpa memanggil jaringan internet sungguhan saat `cargo test`):

1. **`main.rs` (`PresenceTracker`):**
   * Transisi dari `None` $\to$ Aplikasi Terdaftar langsung mengirim update pada siklus ke-1 (*zero-debounce*).
   * Transisi langsung dari Aplikasi A $\to$ Aplikasi B baru mengirim update pada siklus ke-2.
   * Transisi cepat A $\to$ B $\to$ A dalam 1 siklus tidak memicu pengiriman ulang ke Discord.
2. **`config.rs`:**
   * File JSON versi lama (tanpa field `badge_*` dan `default_discord_app_id`) berhasil dimuat menggunakan nilai default (*backward compatibility*).
   * `discord_app_id` kosong (`""`) dinyatakan valid; `discord_app_id` berisi huruf/simbol ditolak.
   * Validasi `logo_key`: string kosong valid, asset key alfanumerik valid, URL `https://` pendek ($< 256\text{ char}$) valid, sedangkan URL `http://` (tidak aman) atau URL dengan spasi ditolak/di-fallback.
3. **`discord_client.rs`:**
   * Payload menyertakan `small_image` dan `small_text` saat `badge_enabled = true`, dan mengosongkannya saat `false`.
   * Saat `app.discord_app_id` kosong, koneksi menggunakan `default_discord_app_id` dan string `details` otomatis memuat `app.app_name`.
4. **`app_resolver.rs`:**
   * Seluruh nama file di daftar hitam (`launcher.exe`, `python.exe`, `setup.exe`, dll) selalu mengembalikan `None` meskipun ada di daftar JSON.
   * Uji skor pencocokan: `blender/blender.exe` menang atas kecocokan nama file biasa; `KambingKuring.exe` tidak cocok secara salah ke `Kambing Simulator` (`kambingsim.exe`).
   * Pembacaan dan penyimpanan cache lokal bekerja dengan benar dan tahan terhadap JSON rusak (tidak `panic!`).
5. **`image_uploader.rs`:**
   * Validasi penolakan ukuran buffer $> 2\text{ MB}$ dan ekstensi file tidak didukung.
   * Simulasi *failover* `ImageHost`: jika Mock Host 1 (Catbox) mengembalikan error, sistem otomatis memanggil Mock Host 2 dan mengembalikan URL `https://` yang valid.
   * Konversi piksel bitmap `BGRA` $\to$ `RGBA` menghasilkan header byte PNG (`\x89PNG\r\n\x1a\n`) yang valid.

---

## 9. Urutan Langkah Implementasi (Roadmap Eksekusi)

1. **Langkah 0 — Verifikasi Awal (Spike):**
   * Siapkan 1 Application ID resmi untuk `larplife` di Discord Developer Portal dan unggah file `larplife_badge.png` ke repositori GitHub.
   * Pastikan URL HTTPS eksternal tampil lancar pada `large_image` dan `small_image`.
2. **Langkah 1 — Pembaruan Engine Inti (`config.rs`, `discord_client.rs`, `main.rs`, `worker.rs`):**
   * Reset versi di `Cargo.toml` menjadi `0.3.0`.
   * Terapkan deteksi detik ke-0, *zero-debounce* pada state kosong, *fallback* `default_discord_app_id`, validasi URL `logo_key`, dan dukungan badge global `larplife` + unit test terkait.
3. **Langkah 2 — Modul Pencari App ID & Uploader Berlapis (`app_resolver.rs` & `image_uploader.rs`):**
   * Tambahkan dependency ringan `ureq` dan `png` di `Cargo.toml`.
   * Bangun `app_resolver.rs` (cache disk 24 jam, normalisasi nama, blacklist, sistem skor) dan `image_uploader.rs` (ekstraksi icon PE/BGRA, *trait* `ImageHost` Catbox + cadangan) lengkap dengan unit test berbasis *mock*.
4. **Langkah 3 — Integrasi GUI Non-Blocking (`settings_gui.rs`) & Checklist Rilis:**
   * Hubungkan fitur pencarian ID dan upload gambar ke `settings_gui.rs` menggunakan *worker thread* + `mpsc::channel` agar UI tetap mulus.
   * Tambahkan banner konfirmasi kandidat ID, tombol pintas browser, dan catatan privasi upload.
   * Jalankan seluruh suite pengujian (`cargo test`), pastikan 0 warning, dan verifikasi RAM daemon tetap $< 10\text{ MB}$ setelah jendela pengaturan ditutup.

---

## 10. Glosarium Singkat
* **Daemon / Worker:** Proses latar belakang `larplife.exe` yang berjalan terus-menerus tanpa jendela untuk memantau aplikasi aktif.
* **Debounce:** Jeda penahanan sementara sebelum mengirim pembaruan status untuk mencegah pengiriman berulang yang terlalu cepat (*spam*).
* **Fallback:** Pilihan atau nilai cadangan yang otomatis dipakai ketika pilihan utama kosong atau mengalami kegagalan.
* **Grace Period:** Batas waktu tunggu ($30\text{ detik}$) sebelum menghapus status Discord saat pengguna berpindah ke aplikasi yang tidak terdaftar.
* **IPC (Inter-Process Communication):** Jalur komunikasi lokal antar-program di Windows menggunakan Named Pipe (`\\.\pipe\discord-ipc-0`) untuk berbicara dengan aplikasi Discord.