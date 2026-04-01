AItermux ProjectYing 外连媒体目录

本目录用于把安卓共享存储中的常用相册目录软链接到项目内，方便 AI 直接访问截图与相册文件。

当前映射：
  - pictures -> /storage/emulated/0/Pictures
  - dcim -> /storage/emulated/0/DCIM
  - camera -> /storage/emulated/0/DCIM/Camera
  - screenshots -> /storage/emulated/0/Pictures/Screenshots
  - game_space_screenshots -> /storage/emulated/0/Pictures/Game Space Screenshot

截图主目录：/storage/emulated/0/Pictures/Screenshots
推荐优先使用：media/screenshots

如果需要让 AI 真正分析图片内容，请调用 view_image(path=...)。
