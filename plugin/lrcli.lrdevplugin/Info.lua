return {
  LrSdkVersion = 14.0,
  LrSdkMinimumVersion = 12.0,
  LrToolkitIdentifier = 'dev.lrcli.bridge',
  LrPluginName = 'lrcli',
  LrInitPlugin = 'Init.lua',
  LrPluginInfoProvider = 'PluginInfoProvider.lua',
  LrForceInitPlugin = true,
  LrLibraryMenuItems = {
    { title = 'lrcli Status…', file = 'StatusMenuItem.lua' },
  },
  VERSION = { major = 0, minor = 1, revision = 0 },
}
