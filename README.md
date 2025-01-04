# POE 2 Filter

Automatic filter updater for POE2, for Linux.

## Usage

In the steam game properties change the command to:

```
poe2filter <sources> -- %command%
```

You can use it before or after your other wrappers, it shouldn't really matter.

### Sources

You can specify any github (more to come/if required) repository that hosts the filter directly in the source tree:

```
poe2filter github:NeverSinkDev/NeverSink-PoE2litefilter -- %command%
```

You can also use one of the builtins:

- [`neversink-lite`](https://github.com/NeverSinkDev/NeverSink-PoE2litefilter)
- [`cdrg`](https://github.com/cdrg/cdr-poe2filter)

As well as multiple values:

```
poe2filter neversink-lite cdrg -- %command%
```

## Debug

This will perform detailed logging. You will typically have to start Steam from the terminal in order to see this.

```
POE2FILTER_LOG=trace poe2filter neversink-lite -- %command%
```

## Cachix

If you're using this on Nix:

```
https://jcdickinson.cachix.org
jcdickinson.cachix.org-1:GZBOGJF64N2yc8z/iAlApnNGgGQv1ApmuMz7xaU5dnY=
```
