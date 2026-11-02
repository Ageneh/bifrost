## Building From Source

When you have these things available, you can install Bifrost by running these commands:

```sh
git clone https://github.com/chrivers/bifrost
cd bifrost
```

Then copy our `config.example.yaml` into a `data` directory:

```sh
mkdir data
cp config.example.yaml data/config.yaml
```

And edit it with your favorite editor to your liking (see
[configuration reference](config-reference.md)).

The `data` directory is mounted into the container and holds everything
Bifrost keeps: `config.yaml`, the draft and backup the web UI writes
(`config.draft.yaml`, `config.yaml.bak`), `state.yaml` with your paired apps
and resource ids, and the certificate. Keep it when you rebuild or update the
container. To keep it somewhere else, change the `./data` path in
`docker-compose.yaml`.

If you set up Bifrost with an earlier `docker-compose.yaml`, which mounted
`config.yaml` and `certs/` on their own, move both into `data/` before you
start the new version:

```sh
mkdir data
mv config.yaml certs data/
```

`state.yaml` was kept inside the container then. To keep your paired apps,
copy it out before you remove the old container:

```sh
docker cp bifrost:/app/state.yaml data/state.yaml
```

Now you are ready to run the app with:

```sh
docker compose up -d
```

This will build and then start the app on your Docker instance.

To view the logs, run the following command:

```sh
docker logs bifrost
```
