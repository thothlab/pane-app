/**
 * Russian translations. Mirror of `en.ts` — same keys, translated
 * values. New keys must be added here alongside their English source;
 * `Dict` from `en.ts` is the type contract.
 */
import type { Dict } from "./en";

const ru: Dict = {
  nav: {
    captures: "Запросы",
    rules: "Правила",
    devices: "Устройства",
    settings: "Настройки",
    docs: "Документация",
    about: "О программе",
    docs_title: "Открыть документацию в браузере",
    filters: "Фильтры",
    delete_filter: "Удалить фильтр",
    delete_filter_confirm: "Удалить фильтр «{name}»?",
    apply_filter: "Применить «{query}»",
  },
  proxy: {
    start: "Запустить прокси",
    stop: "Остановить прокси",
    running: "работает",
    stopped: "остановлен",
  },
  updates: {
    update_to: "Обновить до v{version}",
    installing: "Установка…",
    install_title: "Установить Pane v{version} и перезапустить",
    check_for_updates: "Проверить обновления",
    checking: "Проверка…",
    up_to_date: "У вас последняя версия.",
    server_unreachable: "Не удалось связаться с сервером обновлений. Повторите позже.",
    last_checked: "Последняя проверка: {time}",
  },
  devices: {
    title: "Устройства",
    help_title: "Сопряжение по USB: пошаговая настройка iOS / Android",
    refresh: "Обновить",
    attached_section: "Подключено по USB",
    paired_section: "Сопряжённые",
    no_attached:
      "Устройства не обнаружены. Подключите iPhone или Android, разрешите trust / USB-отладку.",
    no_paired: "Пока нет сопряжённых устройств.",
    add: "Добавить",
    adding: "Добавление…",
    resync: "Пере-синхронизировать",
    resync_title: "Заново применить USB-проброс портов и настройки прокси",
    remove: "Удалить",
    remove_confirm: "Удалить устройство и отозвать настройку?",
    boundaries_title: "Используйте только на своих устройствах.",
    boundaries_body:
      "Pane предназначен для отладки своих приложений и для авторизованной security-работы. Не используйте на устройствах и приложениях, к которым у вас нет прав.",
    tooling_missing_title: "Android-инструменты не найдены",
    almost_there: "Почти готово — завершите установку CA на устройстве.",
    add_failed: "не удалось добавить",
    resync_failed: "не удалось пере-синхронизировать",
    manual_install_toggle: "Как установить сертификат CA",
    manual_install_intro:
      "Ваша сборка Android (чаще всего Samsung One UI на Android 16+) блокирует программную установку CA. Pane уже скопировал сертификат на устройство — закончите установку вручную:",
    manual_install_step1:
      "На телефоне откройте <strong>Настройки → Биометрия и безопасность → Другие параметры безопасности → Установка сертификатов с накопителя → Сертификат ЦС</strong>.",
    manual_install_step2:
      "На предупреждающем экране нажмите <strong>Установить всё равно</strong>.",
    manual_install_step3:
      "В файл-пикере откройте <strong>Внутреннее хранилище → Pane</strong> и выберите <code>pane-ca.pem</code>.",
    manual_install_step4: "Введите PIN/паттерн блокировки экрана.",
    manual_install_lockscreen_note:
      "Без PIN/паттерна блокировки экрана Android не разрешает установку user CA — выставьте блокировку при необходимости. После установки debug-сборки с",
    manual_install_lockscreen_note_after:
      ", доверяющим user CA, начнут принимать Pane. Release-сборки с TLS-пиннингом требуют отдельного обхода.",
    copy_path_title: "Скопировать путь",
  },
  settings: {
    title: "Настройки",
    appearance_section: "Внешний вид",
    theme_label: "Тема",
    theme_system: "Системная",
    theme_light: "Светлая",
    theme_dark: "Тёмная",
    language_label: "Язык",
  },
  about: {
    title: "О программе",
    version_label: "Версия",
    intro:
      "Современный HTTPS-отладчик сетевых запросов, заточенный под одну вещь: <strong>настройка устройства за 30 секунд вместо 15 минут.</strong> Никаких танцев с certificate trust, никакого ручного редактирования Wi-Fi-прокси — подключи iPhone или Android по USB и нажми Add.",
    boundaries_title: "Границы применения",
    boundaries_1:
      "Сделано для отладки <strong>своих</strong> приложений и для авторизованной security-работы.",
    boundaries_2:
      "Не обходит certificate pinning. Когда пиннинг блокирует, вы увидите почему.",
    boundaries_3:
      "Не монитор продакшен-трафика. Не packet-level capture tool.",
    pinning_title: "Cert pinning",
    pinning_para1:
      "Certificate pinning — это защитный механизм, при котором приложение отказывается общаться с сервером, чей сертификат не совпадает с предварительно зашитым отпечатком. Наш MITM-прокси не может выдать себя за такой сервер — by design.",
    pinning_para2:
      "В своих приложениях отключите пиннинг в debug-сборке. Для security-работы на собственных устройствах подойдут Frida или Magisk, которые обходят пиннинг в runtime; Pane их не бандлит.",
    license_title: "Лицензия",
    license_body:
      "Apache-2.0. Построено на rustls, rcgen, libimobiledevice и Android Platform Tools.",
  },
  common: {
    cancel: "Отмена",
    save: "Сохранить",
    delete: "Удалить",
    edit: "Изменить",
    close: "Закрыть",
  },
};

export default ru;
