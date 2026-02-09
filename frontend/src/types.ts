export interface Workload {
   name: string;
   namespace: string;
   image: string;
   current_version: string;
   latest_version: string;
   last_scanned: string;
   update_available: 'Available' | 'NotAvailable' | 'Unknown';
   scan_exhausted: string;
}

export interface Settings {
  system: {
    schedule: string;
    data_dir: string;
    run_at_startup: boolean;
  };
  gitops?: {
    name: string;
    repository_url: string;
  }[];
  notifications?: {
    slack_webhook_url?: string;
    discord_webhook_url?: string;
  };
}
