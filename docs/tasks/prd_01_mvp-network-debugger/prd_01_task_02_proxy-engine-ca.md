# Task 02 — Proxy engine + CA (mitmproxy-rs embed, rcgen, rustls)

## Goal
Запустить локальный HTTPS MITM proxy внутри Tauri-приложения, который терминирует TLS, поднимает leaf-сертификаты на лету от нашего root CA, и эмитит событиe per request/response. Это сердце продукта.

## Scope
**In:**
- Embed `mitmproxy-rs` (или эквивалент) как Rust dependency, обёрнутый в собственный trait `ProxyEngine`.
- Root CA генерация и persistence (PEM на диск, private key в OS keychain).
- Leaf-cert generation on-the-fly (cache в RAM с TTL).
- Lifecycle: start/stop/status, выбор порта.
- Базовая выдача событий `capture.started / capture.completed / capture.error` в IPC bus (без detail body — body sink в Task 03).
- HTTP/1.1 + HTTP/2 support.

**Out:**
- HTTP/3 (QUIC) — только детект ALPN, без proxy.
- WebSocket detail (только маркер).
- Persistence в SQLite — Task 03.
- IPC контракт UI ↔ engine — Task 04 (тут только raw event stream).

## Subtasks

### 2.1 ProxyEngine trait
- [ ] Crate `crates/mycharles-engine/`.
- [ ] Trait:
  ```rust
  #[async_trait]
  pub trait ProxyEngine: Send + Sync {
      async fn start(&self, cfg: EngineConfig) -> Result<EngineHandle>;
      async fn stop(&self, handle: EngineHandle) -> Result<()>;
      fn events(&self) -> broadcast::Receiver<EngineEvent>;
  }
  ```
- [ ] `EngineConfig { listen: SocketAddr, ca: CaMaterial }`.
- [ ] `EngineEvent` enum: `RequestStarted/RequestHeaders/RequestBodyChunk/ResponseHeaders/ResponseBodyChunk/Completed/Error`.

### 2.2 CA management (`crates/mycharles-ca/`)
- [ ] `rcgen` генерация root CA: `CN=my-charles Root CA`, valid 3 years, EKU `serverAuth`+`clientAuth`, key — Ed25519 (fallback RSA-2048 если устройство не поддерживает Ed25519).
- [ ] Сохранение: public PEM + serial + fingerprint в SQLite; private key — через `keyring` крейт.
- [ ] Ротация: новый CA → revoke старый (mark `revoked_at`); leaf cache flush.
- [ ] Экспорт: PEM, DER, mobileconfig (XML profile для iOS), QR (data URL для PEM).

### 2.3 Leaf-cert on-the-fly
- [ ] При новом SNI — issue leaf cert (CN=SNI, SAN с этим же hostname).
- [ ] Cache LRU 1000 entries, TTL 6 часов.
- [ ] Подпись от загруженного root.

### 2.4 mitmproxy-rs adapter
- [ ] Crate `crates/mycharles-engine-mitm/` имплементирует `ProxyEngine` поверх `mitmproxy-rs`.
- [ ] Pin minor version (e.g. `0.X.*`).
- [ ] Mapping `mitmproxy-rs` events → `EngineEvent`.
- [ ] Прокидывание HTTP/2 frame info (для timing waterfall).

### 2.5 Tauri integration
- [ ] State manager: `Arc<dyn ProxyEngine>` + текущий `EngineHandle`.
- [ ] Команды: `proxy.start`, `proxy.stop`, `proxy.status`.
- [ ] Bridge `EngineEvent` → Tauri event bus с де-дупликацией и backpressure (если UI лагает — копим в SQLite, UI догоняет; см. Task 03/04).

### 2.6 Pinning detection hook (предварительный, full UX в Task 12)
- [ ] При TLS handshake failure со стороны клиента сразу после ClientHello — эмит `EngineEvent::PinningSuspected{host, alpn, reason}`.

### 2.7 Performance baseline
- [ ] Бенчмарк: 1000 параллельных HTTPS GET к локальному echo-server через proxy. Целевой throughput ≥ 500 req/s на M1 Air. Замеры в `benches/`.

## Deliverables
- `crates/mycharles-engine/` (trait).
- `crates/mycharles-ca/` (CA + leaf).
- `crates/mycharles-engine-mitm/` (mitmproxy-rs adapter).
- Tauri commands `proxy.start/stop/status`, `ca.current/rotate/export`.
- Benchmark report в `docs/benchmarks/engine-v0.md`.

## Definition of Done
- [ ] `proxy.start` поднимает listener на указанном порту, `proxy.stop` корректно его убивает (нет orphaned thread, нет утечки сокетов).
- [ ] curl через `--proxy 127.0.0.1:8888 --cacert <our-pem>` к произвольному HTTPS host'у возвращает корректный ответ; event'ы видны в logs.
- [ ] HTTP/2 multiplexing работает (2+ stream'а на одном connection).
- [ ] CA generate + rotate + export проходит unit-тесты.
- [ ] Performance baseline ≥ 500 req/s (HTTPS) на dev-машине.
- [ ] Без warnings от clippy в новых крейтах.

## Tests
- **Unit:** rcgen → парс через `x509-parser` → проверка subject/serial/EKU/SAN.
- **Unit:** leaf cache: hit/miss/eviction/TTL.
- **Integration:** `mockito` echo-server → curl через proxy → asserts на event sequence (RequestHeaders → ResponseHeaders → Completed).
- **Integration:** TLS handshake с invalid cert (self-signed pinning симуляция) → `PinningSuspected` event.
- **Bench:** 1000 параллельных запросов, замер latency P50/P95/P99.

## Dependencies
- Task 01 (project bootstrap) — обязателен.
