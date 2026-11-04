# CLAUDE.md

HostDock 探针的开源仓库(2026-11-03 建)。探针是装在用户 Linux 服务器上的一个小程序,定时把 CPU、内存、磁盘、网络这些数报给 HostDock 的探针服务;用户在 HostDock 应用里看实时数据、历史曲线,收告警邮件。

**探针和 HostDock 应用里现有的"监控"(打开服务器时通过 SSH 采集,`../HostDock/crates/hostdock-core/src/monitor.rs`)是两个不相干的功能**(用户 2026-11-03 定):不共用代码,不互相替代,界面上也不合在一起。

## 规矩

- **全部用英文**:代码、注释、文档、提交信息、命令行的输出。这个文件除外(写给我们自己,用中文)。
- **开源的目的是让人审计**:装探针的人要能自己看明白"它只读、不能被远程控制",并且能验证下载的二进制就是这份源码编出来的。所以:
  - **没有下行的命令通道**。探针服务回给探针的内容,探针只认 `docs/PROTOCOL.md` 里列的那几样(上报间隔、服务器时间、令牌无效),别的一律不理。代码里不能出现执行命令、写文件、改配置、自我更新的路径。
  - **只读 `/proc`、`/sys`、`/etc/os-release` 和 `statvfs`**,不调用任何外部命令。不读进程的命令行和环境变量。刻意不采的东西列在 `docs/SECURITY-MODEL.md`,加采集项要先改那里。
  - 不开端口,只出站连编译时写死的探针服务地址(`src/report.rs` 的 `ENDPOINT`)。不支持在配置里改地址(用户 2026-11-03 定,不做自建)。
  - `#![forbid(unsafe_code)]`。依赖尽量少,加依赖要有理由。
  - 令牌不写进日志,也不打到屏幕上。
- **只写已经做到的**:README、`docs/` 里的内容要和代码一致,改了代码回来改文档。不放内部的东西(后端的设计、部署、管理后台)。探针服务的地址本来就编进了探针,可以写。
- **上报协议不适用"开发阶段不管兼容"**:探针不自动更新,装出去的旧版本会一直在。`docs/PROTOCOL.md` 只加字段,不改不删;探针服务要一直接受还在用的旧版本。改协议先改 `docs/PROTOCOL.md`,再改代码,并告诉用户探针服务(`../HostDockServer`)要跟着改。
- 许可是 MIT 或 Apache-2.0 双许可。
- 用户说"提交"才提交,推送同理。做完一步要实际跑过测试,没跑过的要说清楚。

## 命令

```sh
cargo test                     # 解析部分是纯函数,拿 fixtures/ 里真机器上截下来的文件测,在 macOS 上也能跑
cargo clippy --all-targets -- -D warnings
cargo fmt --check

# 真的采一次要在 Linux 上:用 Docker 跑
docker run --rm -v "$PWD":/src -w /src -e CARGO_TARGET_DIR=/src/target/linux rust:1-slim cargo run -q -- once
```

开发时把上报地址指到本机的探针服务:编译时设 `HOSTDOCK_PROBE_ENDPOINT=http://127.0.0.1:18477`(`make run-test` 起的探针服务;Docker 里用 `http://host.docker.internal:18477`)。

## 结构

- `crates/probe/src/collect/`:每类指标一个文件。解析部分是纯函数(输入文件内容,输出结构体),读文件的部分很薄。`mod.rs` 里的 `Collector` 每 5 秒采一次,算速率(按设备分别相减,计数器变小当作从 0 开始)。
- `crates/probe/src/agg.rs`:按整分钟汇总,每一项怎么汇总见 `docs/METRICS.md`。
- `crates/probe/src/report.rs`:上报的格式(和 `docs/PROTOCOL.md` 一一对应)和发送。
- `crates/probe/src/run.rs`:常驻运行。三个线程:采样(主线程)、磁盘空间(每 30 秒,每个挂载点的 `statvfs` 单独一个线程、2 秒超时)、上报。
- `crates/probe/fixtures/`:真机器上截下来的 `/proc` 文件,以及手写的边界情况(没有 MemAvailable 的老内核、OpenVZ 的 venet0 等)。

规划的全文(功能、阶段、探针服务的设计)在 `../HostDockServer/docs/DESIGN.md` 第 17 节;这个仓库里只放公开的部分。
