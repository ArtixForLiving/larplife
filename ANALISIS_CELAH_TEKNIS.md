# Analisis Celah Teknis & Potensi Jebakan — Proyek `larplife` (PRD v2.0 Full Rust)

Dokumen ini merangkum celah arsitektur, edge case sistem operasi Windows, keterbatasan Discord Rich Presence (IPC), serta potensi konflik kebutuhan dari **PRD `larplife` v2.0 — Revisi Full Rust** agar dapat ditinjau lebih mendalam sebelum masuk ke tahap implementasi kode.

---

## 1. Ringkasan Baseline PRD `larplife` (Konteks)
- **Tujuan:** Aplikasi background ringan 100% Rust di Windows 10/11 (64-bit) untuk membaca aplikasi aktif di layar dan mengirim status + logo + nama file/dokumen ke Discord Rich Presence.
- **Arsitektur Modul (4 File):**
  1. `src/config.rs` (`serde`, `serde_json`) — baca & validasi `config.json`.
  2. `src/window_reader.rs` (`windows` crate / Win32 API) — baca nama `.exe` dan judul jendela foreground.
  3. `src/discord_client.rs` (`discord-rich-presence`) — koneksi IPC ke Discord lokal, kirim/hapus status.
  4. `src/main.rs` (`std::thread`) — loop polling tiap 2 detik + deduplikasi pengiriman.
- **Tahapan Pengembangan:**
  - **Tahap 1 (MVP):** Console app + `config.json` (FR-01 s.d. FR-06).
  - **Tahap 2 (Background):** Hilangkan window terminal + System Tray (`Muat Ulang Pengaturan`, `Keluar`) (FR-07).
  - **Tahap 3 (GUI):** Tambah jendela pengaturan berbasis `egui` + deteksi app berjalan (FR-08).
- **Target Performa:** RAM < 5–10 MB, CPU ~0% (menggunakan jeda tidur 2 detik), anti-crash jika Discord belum dibuka, tidak spam RPC jika app & judul file sama.

---

## 2. Daftar Celah Teknis & Potensi Jebakan

### Celah #1: Switching `discord_app_id` & Manajemen IPC Discord (FR-03)
* **Akar Masalah:** Protokol Discord Local IPC (`\\?\pipe\discord-ipc-0` s.d. `9`) mengikat **satu `client_id` (`discord_app_id`)** pada saat handshake awal. Program **tidak dapat** mengganti logo utama maupun nama aplikasi (*"Playing Adobe Photoshop"* $\to$ *"Playing CapCut"*) hanya dengan memanggil `set_activity` ulang pada koneksi yang sama.
* **Konsekuensi Teknis:**
  - Saat berpindah ke aplikasi dengan `discord_app_id` berbeda, `discord_client.rs` wajib menutup koneksi IPC lama secara bersih (`clear_activity` + `close`) lalu membuka koneksi baru dengan `discord_app_id` baru.
  - Jika user sering Alt-Tab bolak-balik antar dua aplikasi terdaftar dalam hitungan detik, siklus *disconnect $\to$ handshake $\to$ set_activity* berulang dapat memicu *rate-limit* atau jeda pembaruan di client Discord.
  - Apabila Discord di-restart, crash, atau user memakai varian **Discord PTB / Canary**, pipe bisa berubah atau terputus mendadak (`BrokenPipe`).
* **Celah UX/Produk:** PRD belum mendefinisikan sumber `discord_app_id` dan `logo_key`. Pengguna awam tidak memiliki Discord Application ID maupun aset gambar yang sudah diunggah ke Discord Developer Portal. Perlu keputusan desain: apakah Larplife menyediakan *preset App ID* bawaan developer atau murni *Bring Your Own App ID*.

---

### Celah #2: Edge Case Win32 API pada Deteksi Window Aktif (FR-01 & FR-04)
Alur standar Win32 (`GetForegroundWindow` $\to$ `GetWindowThreadProcessId` $\to$ `OpenProcess` $\to$ `QueryFullProcessImageNameW` + `GetWindowTextW`) memiliki beberapa jebakan nyata di Windows 10/11:
1. **Privilege Boundary / Elevated Apps (UIPI):**
   - Jika target aplikasi (mis. installer, editor, atau tools tertentu) berjalan sebagai **Administrator** sementara `larplife.exe` berjalan sebagai user standar, `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION)` bisa gagal dengan `ERROR_ACCESS_DENIED`. Program harus menangani kondisi ini tanpa panic/crash.
2. **Aplikasi Microsoft Store / UWP (`ApplicationFrameHost.exe`):**
   - Beberapa aplikasi modern berjalan di dalam container `ApplicationFrameHost.exe`. `GetWindowThreadProcessId` pada top-level window akan mengembalikan `ApplicationFrameHost.exe`, bukan `.exe` aplikasi sebenarnya (membutuhkan enumerasi child window seperti `Windows.UI.Core.CoreWindow` bila ingin didukung).
3. **Foreground Window Sementara / Null HWND:**
   - Saat user menekan tombol Windows (Start Menu), Alt-Tab, volume/media overlay, UAC prompt, atau mengunci layar (`Win+L`), `GetForegroundWindow()` bisa mengembalikan `NULL` (0) atau proses sistem seperti `ShellExperienceHost.exe` / `SearchHost.exe` / `LockApp.exe`.
4. **Case-Sensitivity Nama Proses:**
   - Nama `.exe` dari Win32 path bisa memiliki variasi huruf besar/kecil (`Photoshop.exe` vs `photoshop.exe`). Pencocokan whitelist di `config.rs` wajib bersifat *case-insensitive*.
5. **Multi-Window & Child/Dialog Popup:**
   - Saat aplikasi membuka modal dialog (mis. *Save As*, *Export*), atau saat aplikasi berbasis Electron/Chromium memiliki proses helper, judul jendela bisa berubah sementara menjadi `"Save As"` atau `"Untitled"`.

---

### Celah #3: Konflik Logika FR-05 (Elapsed Time) vs FR-06 (Auto-Clear)
* **Deskripsi Konflik:**
  - **FR-05:** Menampilkan *elapsed time* sejak pengguna membuka/menggunakan aplikasi.
  - **FR-06:** Jika pengguna pindah ke aplikasi non-whitelist (mis. File Explorer), status di Discord otomatis dikosongkan.
  - **Polling 2 detik:** Jika user bekerja di Photoshop lalu membuka File Explorer selama 3 detik untuk mencari referensi/drag file, status akan langsung dihapus lalu muncul kembali dengan timer reset ke `00:00`. Ini menyebabkan efek **flicker** di profil Discord dan merusak akurasi timer kerja.
* **Ambiguitas Timer:**
  - PRD belum menegaskan apakah *elapsed time* dihitung:
    1. **Per sesi aplikasi** (tetap lanjut walau berganti file `a.psd` $\to$ `b.psd`), atau
    2. **Per file/judul jendela** (reset ke `00:00` tiap ganti dokumen), atau
    3. **Sejak proses `.exe` pertama kali berjalan di OS** (`GetProcessTimes`).
  - Selain itu, belum ada mekanisme **grace period / debounce** sebelum status benar-benar dikosongkan (FR-06).

---

### Celah #4: Sanitasi Judul Jendela & Batasan Payload Discord RPC (FR-04)
* **Judul Jendela Berisik:** Banyak aplikasi menyertakan metadata tidak rapi di title bar, misalnya:
  - Photoshop: `poster_event.psd @ 100% (RGB/8) *`
  - Blender: `Blender 4.2 - [D:\Client_Rahasia\project_v2.blend]`
  - Di `config.json` saat ini baru tersedia `"show_window_title": true/false` dan `"default_details"`, belum ada mekanisme *formatting template*, *prefix/suffix stripping*, atau *regex* untuk menghasilkan teks bersih seperti contoh PRD (`"Sedang mengedit: video_liburan.mp4"`).
* **Risiko Privasi:** Tanpa pembersihan path/judul, folder rahasia atau nama klien bisa langsung tampil publik di profil Discord pengguna.
* **Batasan API Discord RPC:**
  - Field `details` dan `state` pada Discord Rich Presence memiliki batas panjang karakter/byte (umumnya minimal 2 karakter dan maksimal ~128 bytes; string kosong atau 1 karakter sering ditolak diam-diam atau menyebabkan update gagal). Perlu pemotongan string UTF-8 yang aman (*char boundary safe truncation*, bukan memotong byte tengah karakter Unicode/emoji).

---

### Celah #5: Kontradiksi Mode Tanpa Terminal (Tahap 2) vs Pelaporan Error (Bagian 7)
* **Akar Masalah:** Bagian 7 mensyaratkan jika `config.json` kosong atau salah ketik, *"program menampilkan pesan pemberitahuan yang jelas mengenai baris mana yang salah"*.
* **Jebakan:** Pada Tahap 2, terminal disembunyikan (umumnya memakai `#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]`). Ketika console tidak ada, `println!` / `eprintln!` tidak akan terlihat oleh user sama sekali—program terlihat seolah "mati diam-diam" (*silent failure*) ketika JSON typo.
* **Kebutuhan yang Hilang:** Perlu mekanisme pelaporan error non-terminal sejak Tahap 2 (mis. Win32 `MessageBoxW` untuk error fatal startup + penulisan ke file log rotasi seperti `larplife.log`).

---

### Celah #6: Arsitektur Concurrency & Threading pada Tahap 2 (System Tray)
* **Masalah:** Di Tahap 1, `main.rs` cukup menjalankan loop `std::thread::sleep(Duration::from_secs(2))`. Namun di Tahap 2:
  - Windows System Tray wajib memompa **Windows Message Loop** di main thread agar ikon tray responsif saat diklik kanan dan tidak dianggap *Not Responding* oleh OS.
  - Artinya loop polling 2 detik **harus dipindah ke worker thread**, dan perlu komunikasi antar-thread untuk aksi *"Muat Ulang Pengaturan"* dan *"Keluar"*.
* **Jebakan `thread::sleep`:** Jika worker thread memakai `std::thread::sleep(2s)` mentah, perintah *"Keluar"* atau *"Muat Ulang Pengaturan"* dari tray bisa tertunda hingga 2 detik, atau tidak bisa melakukan shutdown koneksi Discord secara sinkron sebelum proses exit. Penggunaan channel (`mpsc::Receiver::recv_timeout`) jauh lebih aman daripada `thread::sleep`.

---

### Celah #7: Target Memori < 5–10 MB vs Integrasi `egui` (Tahap 3)
* **Realitas Teknis:**
  - Tahap 1 & 2 (pure Rust + Win32 API) sangat realistis berjalan di kisaran **2–8 MB RAM**.
  - Tahap 3 memilih **`egui`** (umumnya via `eframe` + backend `glow`/OpenGL atau `wgpu`/DX12). Sekali jendela GUI GPU-accelerated dibuka di Windows, driver grafis dan buffer rendering akan menaikkan konsumsi RAM menjadi sekitar **30–80+ MB**—dan pada sebagian allocator/GPU driver, memori tersebut tidak sepenuhnya kembali ke angka < 10 MB meski jendela GUI ditutup.
* **Potensi Solusi untuk Dikaji:** Menjadikan jendela pengaturan sebagai **proses terpisah** (mis. `larplife.exe --settings` yang di-spawn dari Tray lalu exit total setelah user klik Simpan), atau mempertimbangkan GUI native Win32 ringan jika target < 10 MB bersifat mutlak bahkan saat setting dibuka.

---

### Celah #8: Risiko Ekosistem Crate & Blocking I/O
* **Crate `discord-rich-presence`:**
  - Perlu verifikasi status maintenance, error handling saat pipe putus/timeout, serta perilaku saat `close()` vs `clear_activity()`.
  - Operasi named pipe di Windows bisa memblokir (*blocking*) sesaat jika client Discord sedang hang. Jika terjadi di thread yang sama tanpa timeout atau isolasi, siklus polling bisa melambat.
* **Format `config.json` & Tahap 3:**
  - `discord_app_id` pada contoh JSON berupa string numerik (`"1182390192837482910"`). Harus divalidasi hanya berisi digit valid agar tidak error saat handshake.
  - Saat Tahap 3 (GUI) menulis ulang `config.json`, perubahan manual user (seperti urutan atau field tambahan) berisiko tertimpa jika deserialisasi/serialisasi tidak mempertahankan struktur dengan rapi.

---

### Celah #9: Kebutuhan Non-Fungsional yang Belum Tercakup PRD
Beberapa skenario harian di Windows belum tercantum dalam daftar FR-01 s.d. FR-08:
1. **Single Instance Guard:** Mencegah user tidak sengaja menjalankan `larplife.exe` dua kali (yang akan membuat dua koneksi berebut status Discord). Bisa diatasi dengan Windows Named Mutex (`CreateMutexW`).
2. **Sleep / Hibernate / Shutdown Windows:** Saat PC bangun dari mode *Sleep*, pipe IPC ke Discord sering stale/terputus dan timestamp *elapsed* bisa perlu penyesuaian.
3. **Auto-Start Windows:** Aplikasi bertipe tray background umumnya membutuhkan opsi *"Jalankan saat Windows menyala"* (Registry `HKCU\...\Run` atau Startup shortcut).
4. **Aplikasi Berjalan di Background vs Foreground:** PRD memfokuskan deteksi pada *jendela paling depan* (foreground). Jika user sedang mendengarkan Spotify/render Blender sambil buka browser sebentar, status Blender akan hilang. Perlu kejelasan apakah ke depannya akan ada mode *"monitor process in background"* atau tetap murni *"foreground window"*.

---

## 3. Pertanyaan Kunci untuk Analisis Lanjutan (Prompt Guide untuk AI Lain)

Jika dokumen ini diteruskan ke AI lain untuk audit arsitektur atau penyusunan spesifikasi teknis final (`SPEC.md`), fokuskan analisis pada poin-poin berikut:

1. **Strategi Discord IPC:** Bagaimana desain state machine terbaik di `discord_client.rs` untuk menangani perpindahan `discord_app_id` secara cepat tanpa terkena rate-limit Discord, sekaligus tahan terhadap Discord restart/PTB/Canary? Crate Rust mana yang paling stabil saat ini (`discord-rich-presence` vs alternatif)?
2. **Desain Debounce & Timer:** Bagaimana algoritma state tracking di `main.rs` agar FR-05 (elapsed time) dan FR-06 (auto-clear) tidak menyebabkan status berkedip (*flicker*) saat user Alt-Tab singkat ke Explorer atau sistem dialog?
3. **Ekstensi Skema `config.json`:** Field opsional apa saja yang paling minimal namun efektif untuk mendukung sanitasi judul jendela (FR-04) tanpa merusak kesederhanaan `config.json` bagi pengguna Notepad?
4. **Arsitektur Tray + GUI (`egui`):** Apakah sebaiknya Tahap 3 memisahkan GUI ke proses tersendiri (*multi-process / subcommand `--gui`*) demi menjaga target RAM < 10 MB di background, dan bagaimana mekanisme sinkronisasi config reload-nya?
5. **Win32 Edge Cases:** Bagaimana pola implementasi `window_reader.rs` menggunakan crate `windows` (Win32 API) yang aman dari *panic*, *memory leak* handle (`CloseHandle`), UWP (`ApplicationFrameHost.exe`), dan proses *elevated* (`ACCESS_DENIED`)?
