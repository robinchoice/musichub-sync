export function size(bytes) {
  if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toLocaleString('de-DE', { maximumFractionDigits: 1 })} GB`;
  return `${Math.max(1, Math.round(bytes / 1024 ** 2)).toLocaleString('de-DE')} MB`;
}

export function ago(iso) {
  const minutes = Math.round((Date.now() - new Date(iso).getTime()) / 60000);
  if (minutes < 1) return 'gerade eben';
  if (minutes < 60) return `vor ${minutes} Min.`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `vor ${hours} Std.`;
  const days = Math.round(hours / 24);
  return days === 1 ? 'gestern' : `vor ${days} Tagen`;
}

export function agoSecs(secs) {
  return ago(new Date(secs * 1000).toISOString());
}
