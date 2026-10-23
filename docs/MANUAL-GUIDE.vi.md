# RustShot — Hướng dẫn sử dụng

*English: [MANUAL-GUIDE.md](MANUAL-GUIDE.md)*

Tài liệu này hướng dẫn dùng RustShot theo từng việc: cài đặt, chụp và ghi chú lên ảnh, quay
video và GIF, thay đổi cài đặt. Bản tra cứu ngắn gọn (mọi khoá trong `config.toml`, danh sách
đầy đủ tên phím) nằm trong [User Guide](USER-GUIDE.md) (tiếng Anh).

Áp dụng cho **RustShot 0.1.7**. Tên nút và menu trong RustShot là tiếng Anh nên được giữ
nguyên trong tài liệu này.

---

## 1. RustShot làm gì

RustShot là một ứng dụng Windows nhỏ, nằm ở khay hệ thống (các biểu tượng cạnh đồng hồ),
không có cửa sổ chính. Nó làm hai việc:

- **Chụp màn hình**: một vùng tự kéo, một cửa sổ, một màn hình, hoặc tất cả màn hình cùng lúc.
- **Quay màn hình**: quay một vùng hoặc một cửa sổ thành video **MP4** (có thể kèm tiếng micro
  và tiếng của máy tính), hoặc thành ảnh động **GIF**.

Mọi thao tác bắt đầu từ menu ở khay hệ thống hoặc từ phím tắt.

## 2. Cài đặt

RustShot là bản portable: không có trình cài đặt, không cần quyền quản trị.

1. Mở [trang Releases](https://github.com/ngocthien1605/rustshot/releases) và tải file
   `RustShot-<phiên bản>-portable-x64.zip` của bản mới nhất.
2. *(Không bắt buộc)* Kiểm tra file tải về: cạnh file zip có file `.sha256` chứa mã SHA-256.
   Trong PowerShell, lệnh `Get-FileHash .\RustShot-0.1.7-portable-x64.zip` phải in ra đúng mã đó.
3. Giải nén vào một thư mục cố định, ví dụ `%LOCALAPPDATA%\Programs\RustShot`. Trong zip có
   `rustshot.exe`, `USER-GUIDE.md` và `LICENSE`.
4. Nhấp đúp vào `rustshot.exe`.

**"Windows protected your PC".** File exe chưa được ký số, nên lần đầu chạy Windows SmartScreen
có thể cảnh báo. Chọn **More info → Run anyway**.

**Yêu cầu:** Windows 10 bản 1903 trở lên, hoặc Windows 11, 64-bit.

## 3. Lần chạy đầu tiên

- Ở khay hệ thống xuất hiện **biểu tượng vuông bo góc màu xanh ngọc**. Nếu không thấy, bấm mũi
  tên **^** cạnh đồng hồ; có thể kéo biểu tượng ra thanh tác vụ để nó luôn hiện.
- RustShot tạo file cài đặt `%APPDATA%\RustShot\config.toml` và thư mục lưu ảnh/video, mặc định
  là **`Pictures\RustShot`** (nằm trong OneDrive nếu thư mục Pictures được đồng bộ với OneDrive).
- Mỗi lúc chỉ chạy một bản. Mở `rustshot.exe` lần nữa khi nó đang chạy thì không có gì xảy ra.
- Rê chuột lên biểu tượng để xem chú thích. **"RustShot — idle"** nghĩa là mọi phím tắt đều
  dùng được; nếu chú thích báo có phím tắt không dùng được, tức là chương trình khác đang giữ
  phím đó (xem [mục 10](#10-phím-tắt)).

Thoát: biểu tượng ở khay → **Quit RustShot**.

## 4. Menu ở khay hệ thống

Bấm vào biểu tượng ở khay (chuột trái hoặc phải):

| Mục menu | Tác dụng |
|---|---|
| **Screenshot ▸ Screenshot — Region** | Kéo một hình chữ nhật, hoặc bấm vào một cửa sổ, để chụp. |
| **Screenshot ▸ Screenshot — Window** | Bấm vào một cửa sổ để chụp riêng cửa sổ đó. |
| **Screenshot ▸ Screenshot — Monitor** | Chụp cả màn hình đang có chuột. |
| **Screenshot ▸ Screenshot — All monitors** | Chụp tất cả màn hình thành một ảnh. |
| **Record ▸ Record video (region/window)** | Quay một vùng hoặc cửa sổ thành MP4. |
| **Record ▸ Record GIF (region/window)** | Quay một vùng hoặc cửa sổ thành GIF. |
| **Open output folder** | Mở thư mục lưu ảnh và video. |
| **Settings…** | Mở cửa sổ cài đặt ([mục 9](#9-cài-đặt-settings)). |
| **Quit RustShot** | Thoát. Nếu đang quay, video được dừng và lưu trước khi thoát. |

## 5. Chụp màn hình

### Chụp một vùng (phím tắt mặc định `PrintScreen`)

1. Nhấn `PrintScreen`, hoặc khay → **Screenshot ▸ Screenshot — Region**.
2. Màn hình đứng lại và tối đi. Một dấu thập màu trắng đi theo chuột, cửa sổ dưới chuột có viền
   đỏ.
3. **Kéo** một hình chữ nhật (kích thước `W × H` hiện cạnh con trỏ) rồi thả chuột, hoặc **bấm**
   một lần để chụp cửa sổ đang có viền đỏ.
4. Muốn huỷ thì nhấn `Esc` hoặc bấm chuột phải. `Enter` chụp hình chữ nhật đã kéo.

### Chụp một cửa sổ (`Ctrl+PrintScreen`)

Màn hình cũng đứng lại như trên; bấm vào cửa sổ cần chụp. RustShot chụp riêng cửa sổ đó, nên
các cửa sổ khác đè lên nó thường không lọt vào ảnh.

### Chụp một màn hình (`Alt+PrintScreen`) và tất cả màn hình (`Shift+PrintScreen`)

Hai kiểu này chụp ngay, không cần chọn: màn hình đang có chuột, hoặc toàn bộ màn hình nền trên
mọi màn hình.

### Sau khi chụp

- Ảnh được lưu vào thư mục lưu với tên `RustShot_YYYY-MM-DD_HH-mm-ss.png` (hoặc `.jpg` nếu bạn
  chọn JPEG trong Settings). File có sẵn không bao giờ bị ghi đè.
- Ảnh được chép vào clipboard để dán ngay (trừ khi bạn tắt trong Settings). Việc này **thay thế
  nội dung đang có trong clipboard**.
- Một **ô xem trước** nhỏ hiện ở góc phải dưới màn hình trong khoảng 8 giây, và vẫn mở khi chuột
  đang nằm trên nó.

| Nút | Tác dụng |
|---|---|
| **Annotate** | Mở ảnh trong trình ghi chú ([mục 6](#6-ghi-chú-lên-ảnh-annotate)). |
| **Open** | Mở ảnh bằng trình xem ảnh mặc định. |
| **Folder** | Mở File Explorer, chọn sẵn file ảnh. |
| **Copy** | Chép ảnh vào clipboard lần nữa. |
| **Path** | Chép đường dẫn đầy đủ của file. |
| **Delete** | Xoá file và đóng ô xem trước. |

**Chụp hẹn giờ.** Để chụp một menu hay chú thích sẽ biến mất khi bạn nhấn phím, đặt
`delay_seconds` trong `config.toml` (0–60, xem [mục 11](#11-nâng-cao-sửa-configtoml)). RustShot sẽ
chờ chừng ấy giây rồi mới chụp; trong lúc chờ không có gì hiện trên màn hình.

## 6. Ghi chú lên ảnh (Annotate)

1. Trong ô xem trước, bấm **Annotate**. Trình ghi chú mở ra cùng ảnh vừa chụp.
2. Chọn công cụ trên thanh công cụ (cả hai đều vẽ màu đỏ):
   - **Rectangle**: kéo chuột trên ảnh để vẽ khung chữ nhật.
   - **Text**: kéo một khung rồi gõ chữ. **Enter** đặt chữ vào ảnh; **Shift+Enter** hoặc
     **Alt+Enter** xuống dòng; **Backspace** xoá; **Esc** bỏ đoạn chữ đang gõ; bấm ra chỗ khác
     cũng đặt chữ vào ảnh.
3. **Undo** (`Ctrl+Z`) bỏ nét vừa vẽ.
4. **Save** (`Ctrl+S`) lưu một bản sao cạnh ảnh gốc, tên `…_annotated.png`; lưu lần nữa thành
   `…_annotated_2.png`, v.v. **Ảnh gốc không bao giờ bị thay đổi.**
5. **Copy** (`Ctrl+C`) chép ảnh đã ghi chú vào clipboard.

Đóng trình ghi chú khi còn nét chưa lưu thì RustShot hỏi có lưu không. Nét vẽ được đặt theo độ
phân giải gốc của ảnh, dù cửa sổ trình ghi chú nhỏ đến đâu.

## 7. Quay video (MP4)

1. *(Không bắt buộc)* Bật tiếng trong **Settings…** trước ([mục 9](#9-cài-đặt-settings)):
   **Record microphone** (tiếng micro), **Record system audio** (tiếng máy tính đang phát),
   hoặc cả hai.
2. Nhấn `Ctrl+Shift+R`, hoặc khay → **Record ▸ Record video (region/window)**.
3. Kéo một vùng hoặc bấm vào một cửa sổ, giống như khi chụp ảnh. `Esc` để huỷ.
4. Sau **3 giây đếm ngược** (trong lúc đếm không có gì hiện trên màn hình), việc quay bắt đầu
   và một thanh điều khiển nhỏ luôn nằm trên cùng hiện ra, cho biết thời gian đã quay:
   - **Pause / Resume**: tạm dừng / tiếp tục; khoảng tạm dừng không có trong video.
   - **Stop**: kết thúc và lưu file.
5. Bấm **Stop** hoặc nhấn `Ctrl+Shift+S`. File MP4 (hình H.264, tiếng AAC) được lưu vào thư mục
   lưu với tên `RustShot_YYYY-MM-DD_HH-mm-ss.mp4`.

**Dùng micro nào?** Ô **Microphone** trong Settings quyết định điều này: **System default** theo
micro mặc định của Windows; chọn hẳn một micro thì RustShot luôn dùng micro đó, bất kể Windows
đang đặt mặc định là gì. Micro được chọn lúc bắt đầu quay. Xem [mục 9](#9-cài-đặt-settings).

**Khi việc quay tự kết thúc.** Nếu cửa sổ đang quay bị đóng, hoặc driver đồ hoạ khởi động lại,
thanh điều khiển biến mất, phần đã quay được lưu thành file xem được, và một thông báo cho biết
file nằm ở đâu. Thay đổi kích thước cửa sổ đang quay không sao: video giữ kích thước lúc bắt đầu.

**Khi thiếu tiếng.** Nếu micro hoặc tiếng hệ thống không ghi được (chưa cắm, bị rút giữa chừng,
tai nghe Bluetooth bị ngắt), video vẫn được lưu với phần tiếng ghi được, và sau đó có thông báo
liệt kê phần nào bị thiếu và vì sao.

## 8. Quay GIF

1. Khay → **Record ▸ Record GIF (region/window)** (không có phím tắt mặc định).
2. Kéo một vùng hoặc bấm vào một cửa sổ. Sau phần đếm ngược, thanh điều khiển hiện ra.
3. Bấm **Stop**. File GIF được tạo và lưu với tên `RustShot_YYYY-MM-DD_HH-mm-ss.gif`.

GIF được giữ nhỏ có chủ đích: mặc định cạnh dài nhất tối đa **800 px**, **15 khung hình mỗi
giây**, dài tối đa **30 giây**. Chạm giới hạn độ dài thì việc quay tự dừng và có thông báo. GIF
không có tiếng. Các giới hạn này đổi được trong `config.toml` (`gif_max_width`, `gif_fps`,
`gif_max_seconds`).

## 9. Cài đặt (Settings)

Khay → **Settings…**. Cửa sổ này có nút riêng trên thanh tác vụ. Thay đổi chỉ có hiệu lực khi
bấm **OK** (phím tắt được đăng ký lại, thư mục lưu đổi ngay); **Cancel** bỏ mọi thay đổi.

| Mục | Ý nghĩa |
|---|---|
| **Include cursor in screenshots** | Có vẽ con trỏ chuột vào ảnh chụp không. Video mặc định có con trỏ (`recording.include_cursor`). |
| **Copy screenshots to clipboard** | Vừa lưu ảnh vừa chép vào clipboard. |
| **Record microphone** | Ghi tiếng micro vào video. |
| **Microphone** | Chọn micro (mờ đi khi Record microphone đang tắt), xem bên dưới. |
| **Record system audio** | Ghi tiếng máy tính đang phát vào video. |
| **Run RustShot at startup** | Tự chạy RustShot khi đăng nhập Windows. |
| **Save screenshots as JPEG (else PNG)** | JPEG nhẹ hơn; PNG giữ nguyên chất lượng. |
| **JPEG quality (1-100)** | Chất lượng ảnh JPEG; mặc định 90. |
| **Video frame rate (fps)** | Số khung hình mỗi giây của video MP4; mặc định 30. |
| **Output folder** | Thư mục lưu. **Browse…** để chọn thư mục. Để trống nghĩa là `Pictures\RustShot`. |
| **Global hotkeys** | Mỗi thao tác một ô phím tắt ([mục 10](#10-phím-tắt)). |
| **Restore default hotkeys** | Đưa cả sáu phím tắt về mặc định. |

**Danh sách Microphone** gồm:

- **System default (tên micro)**: micro Windows đang đặt làm mặc định, tên ghi trong ngoặc.
  Windows đặt mục này ở *Cài đặt → Hệ thống → Âm thanh → Đầu vào*. Lưu ý: Teams hay Zoom thường
  dùng một mục khác là "thiết bị *liên lạc* mặc định", nên micro chúng dùng có thể không phải
  micro RustShot lấy ở đây.
- **Từng micro đang cắm, theo tên.** Chọn một micro để luôn ghi từ micro đó.
- **Micro đã chọn trước đây nhưng hiện không cắm** vẫn được giữ, kèm chữ **(not connected)**.
  Cho đến khi cắm lại, RustShot dùng micro mặc định và báo điều đó sau khi quay.

**Run at startup** ghi nhớ vị trí của `rustshot.exe`. Nếu chuyển thư mục RustShot sang chỗ
khác, hãy tắt rồi bật lại tuỳ chọn này.

## 10. Phím tắt

| Thao tác | Mặc định |
|---|---|
| Chụp một vùng | `PrintScreen` |
| Chụp một cửa sổ | `Ctrl+PrintScreen` |
| Chụp một màn hình | `Alt+PrintScreen` |
| Chụp tất cả màn hình | `Shift+PrintScreen` |
| Bắt đầu quay (video) | `Ctrl+Shift+R` |
| Dừng quay | `Ctrl+Shift+S` |

**Đổi phím tắt:** Settings → **Global hotkeys** → bấm **Modify** cạnh thao tác, nhấn tổ hợp phím
mới (`Backspace` xoá, `Enter` nhận, `Esc` huỷ), rồi bấm **OK**. Cũng có thể gõ thẳng, ví dụ
`Ctrl+Alt+F9`; để trống một ô thì tắt phím tắt đó. Hai thao tác không được trùng tổ hợp phím, và
tên phím RustShot không hiểu sẽ bị từ chối khi bấm OK, để bạn sửa trước khi lưu.

**Nếu phím tắt không có tác dụng,** tức là chương trình khác đang giữ nó. Trên Windows 11, phím
`PrintScreen` thường mở Snipping Tool: tắt tuỳ chọn đó ở *Cài đặt → Trợ năng → Bàn phím → Dùng
phím Print screen để mở tính năng chụp màn hình*, hoặc thoát công cụ chụp màn hình khác, hoặc
chọn phím tắt khác trong RustShot. Chú thích ở khay cho biết có bao nhiêu phím tắt không dùng
được, và Settings báo nếu phím mới cũng đang bị giữ. Menu ở khay thì luôn dùng được.

## 11. Nâng cao: sửa config.toml

Một số tuỳ chọn chỉ có trong `%APPDATA%\RustShot\config.toml`:

| Khoá | Mặc định | Ý nghĩa |
|---|---|---|
| `screenshot.delay_seconds` | `0` | Chờ trước khi chụp (0–60 giây). |
| `recording.countdown_seconds` | `3` | Đếm ngược trước khi quay (0–30 giây). |
| `recording.include_cursor` | `true` | Vẽ con trỏ chuột vào video. |
| `recording.bitrate` | `8000000` | Chất lượng video, tính bằng bit mỗi giây. |
| `recording.gif_max_width` | `800` | Cạnh dài của GIF, tính bằng pixel (16–7680). |
| `recording.gif_fps` | `15` | Số khung hình mỗi giây của GIF (1–50). |
| `recording.gif_max_seconds` | `30` | Độ dài tối đa của GIF (1–600 giây). |

Cách sửa an toàn:

1. **Thoát RustShot** (khay → Quit RustShot). RustShot đọc file khi khởi động và ghi lại file
   khi bạn bấm OK trong Settings, nên sửa lúc nó đang chạy có thể bị ghi đè.
2. Mở file bằng Notepad, sửa giá trị, lưu lại.
3. Mở lại RustShot.

Giá trị nằm ngoài khoảng cho phép được đưa về mặc định an toàn. Nếu file hỏng đến mức không đọc
được, RustShot đổi tên nó thành `config.toml.broken-<số>`, chạy với cài đặt mặc định và báo cho
bạn. Danh sách đầy đủ các khoá nằm trong [User Guide](USER-GUIDE.md#configtoml-keys).

## 12. RustShot lưu những gì ở đâu

| Nội dung | Vị trí |
|---|---|
| Ảnh chụp và video | `Pictures\RustShot` (hoặc thư mục chọn trong Settings) |
| Cài đặt | `%APPDATA%\RustShot\config.toml` |
| Log (giữ 14 ngày gần nhất) | `%APPDATA%\RustShot\logs\rustshot.log.YYYY-MM-DD` (ngày theo giờ UTC) |
| Chi tiết lỗi nghiêm trọng, nếu có | `%APPDATA%\RustShot\logs\panic.log` |
| Mục tự chạy khi đăng nhập | Registry `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, giá trị `RustShot` |

Để mở thư mục bắt đầu bằng `%APPDATA%`, dán đường dẫn vào thanh địa chỉ của File Explorer.

## 13. Xử lý sự cố

| Vấn đề | Cách xử lý |
|---|---|
| Phím tắt không có tác dụng | Chương trình khác đang giữ phím: xem [mục 10](#10-phím-tắt). |
| Video ghi nhầm micro | Chọn đúng micro ở **Settings → Microphone**, hoặc đổi micro mặc định của Windows. |
| Video không có tiếng | Kiểm tra **Record microphone** / **Record system audio** trong Settings. Nếu thiết bị bị lỗi, thông báo sau khi quay cho biết thiết bị nào và vì sao. |
| Tiếng micro nhỏ | Thường do dùng nhầm micro (ví dụ micro của laptop thay vì micro tai nghe). Chọn đúng micro trong Settings. |
| Ảnh chụp đen hoàn toàn | Cửa sổ đang phát video có bảo vệ bản quyền (DRM), Windows làm đen phần đó khi chụp. File vẫn được lưu và log có ghi lại. |
| "The configured output folder is not usable" | Không tạo được thư mục đã chọn (ổ đĩa không có, không có quyền). Ảnh và video được lưu vào `Pictures\RustShot` cho đến khi bạn chọn thư mục khác. |
| "The settings file could not be read" | `config.toml` bị gõ sai. File cũ được đổi tên thành `config.toml.broken-<số>`; chép lại những gì cần qua Settings rồi xoá file đó. |
| Lỗi khác | RustShot hiện thông báo kèm lý do. Log trong `%APPDATA%\RustShot\logs` có chi tiết; hãy gửi kèm khi báo lỗi. |

Báo lỗi tại [trang GitHub issues](https://github.com/ngocthien1605/rustshot/issues).

## 14. Cập nhật lên bản mới

1. Khay → **Quit RustShot**.
2. Tải file zip mới và thay `rustshot.exe` trong thư mục RustShot.
3. Mở lại. Cài đặt, phím tắt và ảnh/video của bạn vẫn còn (chúng nằm ngoài thư mục chương trình).

## 15. Gỡ bỏ

1. Nếu **Run RustShot at startup** đang bật, tắt nó trong Settings và bấm **OK**.
2. Khay → **Quit RustShot**.
3. Xoá thư mục chương trình RustShot.
4. *(Không bắt buộc)* Xoá `%APPDATA%\RustShot` để xoá cài đặt và log. Ảnh và video trong
   `Pictures\RustShot` không bị đụng tới; tự xoá nếu bạn muốn.
