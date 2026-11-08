# インストール

watch-tower は macOS のホストで常駐し、devcontainer の中の Claude Code からフックで届いたイベントを通知にします。ホストにサーバーを入れたあと、コンテナ側でフックを設定します。

## 必要なもの

- macOS 13 以降
- `cargo` (このリポジトリの `mise.toml` に従って `mise install` で入ります)
- `docker` コマンド (devcontainer CLI を使っていれば入っています)

## ホストにサーバーを入れる

1. ポート 4567 が重なるので、Go 版の container-notify が動いていれば止めます。

   ```sh
   launchctl list | grep -i notif
   launchctl bootout gui/$(id -u)/<ラベル>
   ```

2. ビルドしてインストールします。

   ```sh
   cd <watch-tower のフォルダ>
   make install
   ```

   `WatchTower.app` が `~/Applications` に置かれ、LaunchAgent `dev.watch-tower` として常駐します。ログは `~/Library/Logs/watch-tower.log` に出ます。

3. システム設定 > 通知 で「watch-tower」をオンにして、再起動します。初回は通知がオフのまま登録されることがあります。

   ```sh
   make restart
   ```

4. 動いているか確かめます。

   ```sh
   curl http://localhost:4567/healthz
   ```

   `ok` が返れば動いています。

`docker` が見つからないと言われたときは、場所を指定します。

```sh
make install DOCKER=/path/to/docker
```

## コンテナ側でフックを設定する

1. `hooks/claude-hook.sh` を、コンテナの `/template` にマウントしているホストのフォルダに `watch-tower-hook.sh` としてコピーします。シンボリックリンクはコンテナから辿れないので使いません。

   ```sh
   cp hooks/claude-hook.sh <テンプレートのフォルダ>/watch-tower-hook.sh
   chmod +x <テンプレートのフォルダ>/watch-tower-hook.sh
   ```

2. ホストの `~/.claude/settings.json` の `hooks` を次のようにします。スクリプトがまだ見えないコンテナでは、今までの `/template/notify.sh` が動きます。

   ```json
   "hooks": {
     "SessionStart": [
       { "hooks": [ { "type": "command", "command": "if [ -x /template/watch-tower-hook.sh ]; then exec /template/watch-tower-hook.sh; fi" } ] }
     ],
     "Notification": [
       { "hooks": [ { "type": "command", "command": "if [ -x /template/watch-tower-hook.sh ]; then exec /template/watch-tower-hook.sh; else exec /template/notify.sh; fi" } ] }
     ],
     "Stop": [
       { "hooks": [ { "type": "command", "command": "if [ -x /template/watch-tower-hook.sh ]; then exec /template/watch-tower-hook.sh; else exec /template/notify.sh; fi" } ] }
     ],
     "SessionEnd": [
       { "hooks": [ { "type": "command", "command": "if [ -x /template/watch-tower-hook.sh ]; then exec /template/watch-tower-hook.sh; fi", "timeout": 5 } ] }
     ]
   }
   ```

3. コンテナで Claude Code を新しく起動します。フックの設定はセッションの開始時に読まれます。

4. コンテナの中から、セッションが登録されたか確かめます。

   ```sh
   curl http://host.docker.internal:4567/sessions
   ```

## 更新

```sh
git pull
make install
```

`make install` は常駐中のサーバーを止めてから入れ替えます。通知の許可はそのまま残ります。

## アンインストール

```sh
make uninstall
```

`WatchTower.app` と LaunchAgent を消します。記録 (`~/Library/Application Support/watch-tower/`) とログは残ります。

## うまくいかないとき

| 症状 | 確かめること |
|---|---|
| 通知が出ない | システム設定 > 通知 で「watch-tower」がオンか。ログに `notifications are off` が出ていないか |
| `curl http://localhost:4567/healthz` に応答がない | ログに `failed to listen` が出ていないか。Go 版のサーバーなど、ポート 4567 を使うものが残っていないか |
| プロジェクト名がコンテナ ID になる | ログに `docker inspect` のエラーが出ていないか。`make install DOCKER=...` で docker の場所を指定したか |
| セッション名が ID になる | コンテナの `/template/watch-tower-hook.sh` が実行できるか |

ログは次のコマンドで追えます。

```sh
make logs
```
