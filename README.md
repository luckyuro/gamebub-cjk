<p align="center">
  <a href="https://gamebub.net/">
    <img src="./assets/logo.svg" width="480" alt="Game Bub logo">
  </a>
</p>

[![Game Bub trailer](./assets/video-poster.jpg)](https://www.youtube.com/watch?v=f16E5J6qljw)

**Game Bub** is an open-source FPGA retro emulation handheld, with support for Game Boy, Game Boy Color, and Game Boy Advance games.

You can buy your own, prebuilt Game Bub from **[Crowd Supply](https://www.crowdsupply.com/second-bedroom/game-bub)**!

You can find the [user guide and documentation here](https://docs.gamebub.net/).

Want to chat about Game Bub? Feel free to [join the Discord](https://discord.gg/T5xrYpMfN7).

Check out the [announcement blog post](https://eli.lipsitz.net/posts/introducing-gamebub/) for an in-depth look at the development process!


## Features
* Play physical Game Boy / Color / Advance cartridges
* Load and play ROM files from a microSD card (with built-in support for rumble, clock, accelerometer, gyroscope)
* Multiplayer link cable functionality
* Custom, from-scratch Game Boy and Game Boy Advance FPGA cores with great game compatibility
* 14+ hour battery life 
* Video output to TV or monitor via Game Bub Dock
* Extensible hardware, designed for future improvements

## Building

Building a Game Bub handheld requires manufacturing PCBs, 3D printing the shell and buttons, and assembling components from a variety of sources. For information on manufacturing and assembling your own, see [here](https://github.com/elipsitz/gamebub/blob/v0.1/docs/building.md). Note that this information is for the older, vertical revision 2. Updated building information will be available in the Game Bub Docs soon.

For other inquiries, contact us at support@gamebub.net.

## Architecture

For an in-depth description of the project architecture, as of revision 2, see [here](https://github.com/elipsitz/gamebub/blob/v0.1/docs/architecture.md).

The Game Bub handheld consists of a Xilinx XC7A100T FPGA to do the main emulation and I/O, and an ESP32-S3 microcontroller to do auxiliary tasks (configuring the FPGA, rendering the UI, loading ROM files from a microSD card and sending it to the FPGA).

### Directory Structure

* `fpga`: FPGA source code (HDL), written in [Chisel](https://github.com/chipsalliance/chisel)
* `firmware`: Microcontroller firmware

## License

Unless otherwise specified:

* Firmware (in `firmware/`) and scripts (in `scripts/`) are licensed under GPLv3 (`GPL-3.0-only`).
* FPGA source code (in `fpga/`) is licensed under the CERN Open Hardware License Version 2 - Strongly Reciprocal (`CERN-OHL-S-2.0`)
* PCB (schematic and layout), mechanical, and hardware design files are licensed under the CERN Open Hardware License Version 2 - Strongly Reciprocal (`CERN-OHL-S-2.0`)

At a high level, this means that you can copy, share, and modify the source code, as long as you provide proper attribution and share your source code / design files with the same license. However, the "Game Bub" name and logo are trademarked, and you may not use them for your product without permission.

## 核心文件列表编译选项

已适配上游 `handheld` 的 CoreManager 架构；各核心按自身配置筛选文件，
并保留各文件槽上次选择的路径。源码版本为 `1.0.2-fusionpixel.3`。

在 `firmware/handheld` 中编译（下面以 rev4 为例）：

```sh
# 默认：上下选择；左右键或 L/R 翻页，每页 10 个文件／目录
./build_fusion_pixel_full.sh --release --features=rev4

# 滚动：上下选择，到当前批次边界时自动加载前／后一批
./build_fusion_pixel_full.sh --release --features=rev4,rom-list-scroll
```

两种模式都不显示 Previous/Next page 条目。翻页模式每页最多 10 个文件／目录，
并为子目录的“..”返回项预留一行，可在一屏内完整显示；首尾页不能继续向外翻页。
滚动模式在列表首尾停止；跨批次会短暂显示 Loading。
两种模式都支持 A 打开、B 返回上级目录及恢复上次选中的文件。

两种模式共用有界目录扫描：翻页模式每页最多 10 项，扫描堆最多保留 11 个候选，
UI 最多 11 行（含上级目录）；滚动模式每批最多 32 项，扫描堆最多保留 33 个候选，
UI 最多 33 行。加载前清空旧模型，
不缓存或追加历史批次，因此列表内存不会随目录条目总数增长。
文件名保持完整用于打开文件，显示宽度限制为 250px；全量中文字库覆盖文本
仍只在编译时使用。此约束避免全目录加载导致的 OOM；设备整体剩余堆、
渲染缓存和长期运行仍需实机压力测试确认。

回归及内存测试见 [Core file selector host checks](firmware/handheld/tests/rom-selector-host/README.md)。

### UF2 打包

新版固件必须同时更新主程序与匹配的 `system_data` 分区。当前基于上游
`v1.0.2`（`e779b7c`），使用该版本官方 rev4 UF2 中的完整 2 MiB 系统数据，
包含匹配的 FPGA 核心及 BIOS 资源。不要沿用旧版仅含主程序的 UF2 打包方式。

```sh
espflash save-image --chip esp32s3 --flash-size 8mb --flash-freq 80mhz \
  --partition-table firmware/handheld/partitions.csv --target-app-partition factory \
  path/to/handheld path/to/application.bin
python3 firmware/handheld/make_uf2.py path/to/application.bin path/to/output.uf2 \
  --system-data path/to/system-data-v1.0.2.bin
```

`gamebub-cjk-rev4-paged.uf2` 为按键翻页版，
`gamebub-cjk-rev4-scroll.uf2` 为连续滚动版；
`gamebub-cjk-rev4.uf2` 与分页版相同。构建来源和校验值见
`firmware/handheld/uf2-build-info.json` 与根目录 `SHA256SUMS`。
本次 `v1.0.2-fusionpixel.3-paged` Release 单独发布翻页版，新增 BIOS 检测和加载开关；
仓库中的滚动版固件仍为 `1.0.2-fusionpixel.1`。
两种 UF2 仅写入主程序和系统数据分区，不包含 NVS 或 DFU 分区。

核心生命周期及故障模拟见 [Core host checks](firmware/handheld/tests/core-host/README.md)。

### BIOS 检测与加载选择

`Settings → Core: GB / GBC` 和 `Settings → Core: GBA` 增加了
`Use SD BIOS (off: built-in)` 开关，默认开启，保留原来的自动加载行为。
关闭后，即使 SD 卡上存在 BIOS，也使用固件自带的替代 BIOS。
GB / GBC 共用一个开关，GBA 单独设置；设置会保存，在下次启动游戏或卡带时生效。

页面同时显示 SD 卡 BIOS 的检测结果：`Found`（找到且大小符合）、
`Not found`（未找到）、`Wrong size`（大小不符）或 `Read error`（读取失败）。
每次进入对应配置页会重新检测；这里只检查文件类型和大小，不校验 BIOS 内容。

| 核心 | SD 卡文件 | 大小 |
| --- | --- | --- |
| GB | `/system/gameboy.bios-dmg.bin` | 256 字节 |
| GBC | `/system/gameboy.bios-cgb.bin` | 2,304 字节 |
| GBA | `/system/gba.bios.bin` | 16,384 字节 |

上表路径相对于 SD 卡根目录。已有的 GBA 跳过开机动画配置若启用，
则检测和加载 `/system/gba.bios-fast.bin`（同为 16,384 字节）。
开启 SD BIOS 时，文件缺失会自动使用内置替代 BIOS，大小不符仍由加载器报错；
主动关闭时不再弹出 GBA 缺少 BIOS 的提醒。内置替代 BIOS 的游戏兼容性可能与外部 BIOS 不同。
