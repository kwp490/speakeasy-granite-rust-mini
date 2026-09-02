export function SettingsPageHeader({
  detail,
  eyebrow,
  title,
}: {
  detail: string;
  eyebrow: string;
  title: string;
}) {
  return (
    <header className="settings-page-header">
      <p className="eyebrow">{eyebrow}</p>
      <h2>{title}</h2>
      <p>{detail}</p>
    </header>
  );
}
