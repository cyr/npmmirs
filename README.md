# npmmirs

**npmmirs** is a simple tool to mirror NPM packages into a self-hosted repository for offline use.

Note that **npmmirs** does not attempt to do a minimal-complete mirror of the seed packages in your package.json files. It will do a complete resolution of all dependencies and child-dependencies, which typically will result in quite a large (but complete!) tree of dependencies. You can optionally not mirror dev-dependencies, peer-dependencies and optional-dependencies via the appropriate switch.

However, any non-npm dependency will not be pulled down - such as git-repository data or http(/)s paths.

## Usage

**npmmirs** needs a manifests path containing files with the .json extension that adhere to the packages.json format. These will server as the seed-packages when resolving dependencies.

`./npmmirs --manifests-path ./manifests --output /opt/npm/output`

The default mode of operations will mirror all dependencies, dev-dependencies, peer-dependencies and optional-dependencies as per the highest matching version of each specified range. There is also a `--greedy` switch that will change this to *all matching versions*.

`./npmmirs --greedy --manifests-path ./manifests --output /opt/npm/output`

Using this will result in a lot of tarballs being pulled down, but will probably result in a more complete mirror - but is probably not necessary, unless having historical old versions is important to you.

## Hosting

The output folder is structured in the same way as the official registry.npmjs.org. To host this, just set up a web server (such as nginx) to point to the output folder, adding index.json as the index file, serving application/json content.

Note that tarball-paths are not rewritten in the metadata file, so they will be expected to be downloaded from https://registry.npmjs.org/{package_name}/-/{tarball}

## Example nginx.conf

```
server {
    listen 443;
    server_name registry.npmjs.org;

    ssl_certificate     self-signed-registry.npmjs.org.crt;
    ssl_certificate_key self-signed-registry.npmjs.org.key;
    ssl_protocols       TLSv1.2 TLSv1.3;
    ssl_ciphers         HIGH:!aNULL:!MD5;
    
    index index.json;
    
    location / {
        autoindex on;
        root /opt/npm/output/;
    }
}  
```