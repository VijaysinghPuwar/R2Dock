export function HelpPanel() {
  return (
    <section className="help-panel" aria-labelledby="help-title">
      <h3 id="help-title" className="help-title">
        Help finding credentials
      </h3>
      <ol className="help-steps">
        <li>Open the Cloudflare dashboard and go to R2.</li>
        <li>Find your Account ID on the R2 overview page.</li>
        <li>Select "Manage API tokens" and create a token.</li>
        <li>
          Choose "Object Read &amp; Write" and pick the buckets R2Dock should manage. Choose "Admin" permissions only if you
          need to see all buckets.
        </li>
        <li>Copy the Access Key ID and Secret Access Key. The secret is shown only once.</li>
      </ol>
      <p className="help-note">
        R2 S3 credentials are different from Cloudflare API tokens used by other tools. R2Dock only needs the S3 credentials.
      </p>
    </section>
  );
}
