---
title: Getting started
description: Install r3cui on your Unfolded Circle Remote 3, and go back to the Normal UI.
---

:::danger[These be treacherous waters]
Installing r3cui voids your Remote 3 warranty. The remote keeps a permanent record that a custom UI was installed.
That record stays even if you go back to the Normal UI.
:::

## What you need

- An [Unfolded Circle Remote 3](https://www.unfoldedcircle.com/) with firmware 2.8.4 or newer (the version r3cui is
  tested on).
- [Kodi](https://kodi.tv) and [Home Assistant](https://www.home-assistant.io), both already set up on your remote.
- A receiver (amplifier) that Home Assistant can control.
- A computer on the same network as the remote.
- Your remote's IP address and web configurator PIN. You can find both in the remote's settings.

## 1. Download

Download `remote-ui.tar.gz` from the [latest release](https://github.com/jackhric/r3cui/releases/latest).

## 2. Install

Open a terminal in the folder with the file and run this command. Put your PIN and IP address in place of `<PIN>`
and `<IP>`.

```sh
curl -u 'web-configurator:<PIN>' \
  'http://<IP>/api/system/install/ui?void_warranty=yes' \
  --form 'file=@remote-ui.tar.gz'
```

## 3. Wait

Wait for the remote to show the new screen. If it does not, restart the remote.

## Go back to the Normal UI

```sh
curl -u 'web-configurator:<PIN>' -X PUT 'http://<IP>/api/system/install/ui?enable=false'
```

To remove r3cui completely:

```sh
curl -u 'web-configurator:<PIN>' -X DELETE 'http://<IP>/api/system/install/ui'
```

If r3cui fails to start many times in a row, the remote goes back to the Normal UI by itself.
