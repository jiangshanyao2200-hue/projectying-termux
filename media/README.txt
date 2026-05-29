AItermux ProjectYing 外连媒体目录

本目录只维护安卓相机与截图目录的软链接，方便 AI 直接访问照片与最近截图。

当前映射：
  - dcim -> /storage/emulated/0/DCIM
  - camera -> /storage/emulated/0/DCIM/Camera
  - screenshot -> /storage/emulated/0/Pictures/Screenshots

相册主目录：/storage/emulated/0/DCIM
相机子目录：/storage/emulated/0/DCIM/Camera
截图主目录：/storage/emulated/0/Pictures/Screenshots
推荐优先使用：media/screenshot/ （传目录时会自动选择最新截图）
照片可直接使用：media/dcim/

如果需要让 AI 真正分析图片内容，请调用 view_image(path=...)。
