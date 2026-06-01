---
title: Фильтрация captures
description: Маленький DSL в строке поиска — ключи, диапазоны, globs, отрицание, barewords.
---

Строка поиска в списке captures принимает маленький filter DSL.
Парсер идёт слева направо, все термы через пробел должны совпасть (AND).

## Ключи

| Ключ | С чем матчится | Пример |
| --- | --- | --- |
| `host:` | substring `server_host` (SQL `LIKE`, `*` работает) | `host:api.example.com`, `host:*.dev` |
| `method:` | точный method, case-insensitive | `method:POST` |
| `status:` | одиночный status или диапазон | `status:200`, `status:500..599`, `status:5..` |
| `mime:` | substring response `Content-Type` | `mime:json`, `mime:image/` |
| `path:` | substring URL path (`*` работает) | `path:/v1/*`, `path:auth` |
| `size:` | размер response в байтах, одиночный или диапазон | `size:0`, `size:1000..` |
| `duration:` | длительность запроса в мс | `duration:1000..` (медленные), `duration:..50` (быстрые) |
| `error:` | точное значение `error_kind` | `error:tls_handshake`, `error:pinning` |

Ключи **case-insensitive** — `host:`, `Host:` и `HOST:` сводятся к
одному и тому же. Удобно, когда iOS автокапитализирует первую букву.

## Отрицание

Префикс `!` исключает совпадения:

```text
!error:tls_handshake          # выбросить всё, что упало по TLS
!host:*.cdn.example.com       # игнорировать CDN-шум
!path:/healthz                # скрыть health-check'и
```

## Barewords

Терм без двоеточия — substring-поиск одновременно по **host или path**:

```text
google                        # любой capture, где google.com или /google
docs                          # matches host:docs.example.com OR path:/docs
```

Фразу с пробелами или спецсимволами — в кавычки: `"some phrase"`.

## Сохранение фильтров

Кнопка ☆ справа от строки поиска сохраняет текущий фильтр в sidebar.
Сохранённые фильтры переживают перезапуск приложения.

## Подсветка синтаксиса

Токены подсвечиваются по мере ввода:

| Цвет | Значение |
| --- | --- |
| accent (синий) | Известный ключ (`host`, `method`, …) |
| красный, dotted underline | Неизвестный ключ — backend отбросит этот терм |
| красный | Префикс отрицания `!` |
| muted | Разделитель `:` |
| default | Значения и barewords |

Неизвестные ключи флагуются сразу, до отправки в backend.

## Чего пока нет

- Нет `OR` между термами. Workaround: сохранить два фильтра и
  переключаться.
- Нет regex (намеренно — DSL для skimming, не для grep).
- Нет поиска внутри тела ответа. Используй Tree-режим в body viewer
  для навигации по перехваченным телам.
