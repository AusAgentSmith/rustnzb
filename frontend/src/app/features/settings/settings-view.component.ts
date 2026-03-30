import { Component, OnInit, signal } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatButtonModule } from '@angular/material/button';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { ApiService } from '../../core/services/api.service';

interface ServerConfig {
  id: string; name: string; host: string; port: number; ssl: boolean; ssl_verify: boolean;
  username: string | null; password: string | null; connections: number; priority: number;
  enabled: boolean; retention: number; pipelining: number; optional: boolean; compress: boolean;
  ramp_up_delay_ms: number; proxy_url: string | null;
}

interface CategoryConfig {
  name: string; output_dir: string | null; post_processing: number;
}

function emptyServer(): ServerConfig {
  return {
    id: '', name: '', host: '', port: 563, ssl: true, ssl_verify: true,
    username: null, password: null, connections: 8, priority: 0,
    enabled: true, retention: 0, pipelining: 16, optional: false, compress: false,
    ramp_up_delay_ms: 250, proxy_url: null,
  };
}

function emptyCategory(): CategoryConfig {
  return { name: '', output_dir: null, post_processing: 3 };
}

@Component({
  selector: 'app-settings-view',
  standalone: true,
  imports: [CommonModule, FormsModule, MatButtonModule, MatSnackBarModule],
  template: `
    <div class="settings-layout">
      <div class="nav">
        <div class="nav-item" [class.active]="tab === 'servers'" (click)="tab = 'servers'">Servers</div>
        <div class="nav-item" [class.active]="tab === 'categories'" (click)="tab = 'categories'">Categories</div>
        <div class="nav-item" [class.active]="tab === 'general'" (click)="tab = 'general'">General</div>
      </div>
      <div class="content">

        <!-- ==================== SERVERS TAB ==================== -->
        @if (tab === 'servers') {
          <div class="tab-header">
            <h3>News Servers</h3>
            @if (!editingServer) {
              <button class="btn btn-primary" (click)="addServer()">Add Server</button>
            }
          </div>

          @if (editingServer) {
            <div class="form-card">
              <h4>{{ editingServerId ? 'Edit Server' : 'Add Server' }}</h4>
              <div class="form-grid">
                <div class="form-row">
                  <label>Name</label>
                  <input type="text" [(ngModel)]="editingServer.name" placeholder="My Server" />
                </div>
                <div class="form-row">
                  <label>Host</label>
                  <input type="text" [(ngModel)]="editingServer.host" placeholder="news.example.com" />
                </div>
                <div class="form-row">
                  <label>Port</label>
                  <input type="number" [(ngModel)]="editingServer.port" />
                </div>
                <div class="form-row">
                  <label>SSL</label>
                  <input type="checkbox" [(ngModel)]="editingServer.ssl" />
                </div>
                <div class="form-row">
                  <label>Verify SSL</label>
                  <input type="checkbox" [(ngModel)]="editingServer.ssl_verify" />
                </div>
                <div class="form-row">
                  <label>Username</label>
                  <input type="text" [(ngModel)]="editingServer.username" placeholder="(optional)" />
                </div>
                <div class="form-row">
                  <label>Password</label>
                  <input type="password" [(ngModel)]="editingServer.password" placeholder="(optional)" />
                </div>
                <div class="form-row">
                  <label>Connections</label>
                  <input type="number" [(ngModel)]="editingServer.connections" min="1" />
                </div>
                <div class="form-row">
                  <label>Priority</label>
                  <input type="number" [(ngModel)]="editingServer.priority" min="0" />
                </div>
                <div class="form-row">
                  <label>Retention (days)</label>
                  <input type="number" [(ngModel)]="editingServer.retention" min="0" />
                </div>
                <div class="form-row">
                  <label>Pipelining</label>
                  <input type="number" [(ngModel)]="editingServer.pipelining" min="0" />
                </div>
                <div class="form-row">
                  <label>Enabled</label>
                  <input type="checkbox" [(ngModel)]="editingServer.enabled" />
                </div>
                <div class="form-row">
                  <label>Optional (backup)</label>
                  <input type="checkbox" [(ngModel)]="editingServer.optional" />
                </div>
                <div class="form-row">
                  <label>Compress</label>
                  <input type="checkbox" [(ngModel)]="editingServer.compress" />
                </div>
              </div>
              <div class="form-actions">
                <button class="btn btn-primary" (click)="saveServer()">Save</button>
                <button class="btn" (click)="cancelServerEdit()">Cancel</button>
              </div>
            </div>
          }

          @for (s of servers(); track s.id) {
            <div class="server-card">
              <div class="server-info">
                <div class="server-name">{{ s.name }} @if (s.optional) { <span class="optional">(backup)</span> }</div>
                <div class="server-host">{{ s.host }}:{{ s.port }} · {{ s.ssl ? 'SSL' : 'Plain' }} · {{ s.connections }} conn · Pipeline: {{ s.pipelining }}</div>
              </div>
              <div class="server-status">
                <span class="dot" [class.active]="s.enabled"></span>
                {{ s.enabled ? 'Enabled' : 'Disabled' }}
              </div>
              <button class="btn" (click)="editServer(s)">Edit</button>
              <button class="btn" (click)="testServer(s.id)">Test</button>
              <button class="btn btn-danger" (click)="deleteServer(s.id)">Delete</button>
            </div>
          }
          @if (servers().length === 0 && !editingServer) {
            <div class="empty">No servers configured</div>
          }
        }

        <!-- ==================== CATEGORIES TAB ==================== -->
        @if (tab === 'categories') {
          <div class="tab-header">
            <h3>Categories</h3>
            @if (!editingCategory) {
              <button class="btn btn-primary" (click)="addCategory()">Add Category</button>
            }
          </div>

          @if (editingCategory) {
            <div class="form-card">
              <h4>{{ editingCategoryOriginalName ? 'Edit Category' : 'Add Category' }}</h4>
              <div class="form-grid">
                <div class="form-row">
                  <label>Name</label>
                  <input type="text" [(ngModel)]="editingCategory.name" placeholder="movies" />
                </div>
                <div class="form-row">
                  <label>Output Directory</label>
                  <input type="text" [(ngModel)]="editingCategory.output_dir" placeholder="(optional)" />
                </div>
                <div class="form-row">
                  <label>Post Processing</label>
                  <select [(ngModel)]="editingCategory.post_processing">
                    <option [ngValue]="0">None</option>
                    <option [ngValue]="1">Repair</option>
                    <option [ngValue]="2">Unpack</option>
                    <option [ngValue]="3">Repair + Unpack</option>
                  </select>
                </div>
              </div>
              <div class="form-actions">
                <button class="btn btn-primary" (click)="saveCategory()">Save</button>
                <button class="btn" (click)="cancelCategoryEdit()">Cancel</button>
              </div>
            </div>
          }

          @for (c of categories(); track c.name) {
            <div class="server-card">
              <div class="server-info">
                <div class="server-name">{{ c.name }}</div>
                <div class="server-host">{{ c.output_dir || 'Default output' }} · {{ ppLabel(c.post_processing) }}</div>
              </div>
              <button class="btn" (click)="editCategory(c)">Edit</button>
              <button class="btn btn-danger" (click)="deleteCategory(c.name)">Delete</button>
            </div>
          }
          @if (categories().length === 0 && !editingCategory) {
            <div class="empty">No categories configured</div>
          }
        }

        <!-- ==================== GENERAL TAB ==================== -->
        @if (tab === 'general') {
          <h3>General Settings</h3>
          <div class="setting-row">
            <label>Speed Limit (bytes/sec, 0 = unlimited)</label>
            <input type="number" [(ngModel)]="speedLimit" class="setting-input" />
            <button class="btn" (click)="saveSpeedLimit()">Save</button>
          </div>
          <div class="setting-row">
            <label>Max Concurrent Downloads</label>
            <input type="number" [(ngModel)]="maxActiveDownloads" class="setting-input" min="1" />
            <button class="btn" (click)="saveMaxActive()">Save</button>
          </div>
          <div class="setting-row">
            <label>History Retention (days, blank = keep all)</label>
            <input type="number" [(ngModel)]="historyRetention" class="setting-input" min="0" />
            <button class="btn" (click)="saveRetention()">Save</button>
          </div>
        }

      </div>
    </div>
  `,
  styles: [`
    :host { display: flex; height: 100%; }
    .settings-layout { display: flex; flex: 1; overflow: hidden; }
    .nav { width: 180px; border-right: 1px solid #21262d; background: #0d1117; padding: 8px 0; }
    .nav-item { padding: 8px 16px; cursor: pointer; font-size: 13px; color: #8b949e; }
    .nav-item:hover { color: #c9d1d9; background: #161b22; }
    .nav-item.active { color: #c9d1d9; background: rgba(56,139,253,0.1); border-right: 2px solid #58a6ff; }
    .content { flex: 1; padding: 20px 24px; overflow-y: auto; }
    h3 { font-size: 16px; margin-bottom: 12px; }
    .tab-header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 12px; }
    .tab-header h3 { margin-bottom: 0; }

    /* Server / category cards */
    .server-card { display: flex; align-items: center; gap: 12px; padding: 12px 16px; background: #161b22; border: 1px solid #30363d; border-radius: 6px; margin-bottom: 8px; }
    .server-info { flex: 1; }
    .server-name { font-weight: 600; font-size: 14px; }
    .optional { color: #8b949e; font-weight: normal; font-size: 12px; }
    .server-host { font-size: 12px; color: #8b949e; margin-top: 2px; }
    .server-status { display: flex; align-items: center; gap: 4px; font-size: 12px; color: #8b949e; }
    .dot { width: 8px; height: 8px; border-radius: 50%; background: #484f58; }
    .dot.active { background: #3fb950; }

    /* Buttons */
    .btn { padding: 4px 10px; border-radius: 4px; border: 1px solid #30363d; background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; }
    .btn:hover { background: #30363d; }
    .btn-primary { background: #238636; border-color: #238636; color: #fff; }
    .btn-primary:hover { background: #2ea043; }
    .btn-danger { color: #f85149; }
    .btn-danger:hover { background: #30363d; }

    /* Inline form card */
    .form-card { background: #161b22; border: 1px solid #30363d; border-radius: 6px; padding: 16px 20px; margin-bottom: 16px; }
    .form-card h4 { font-size: 14px; margin: 0 0 12px 0; }
    .form-grid { display: flex; flex-wrap: wrap; gap: 10px 24px; }
    .form-row { display: flex; align-items: center; gap: 8px; }
    .form-row label { font-size: 13px; min-width: 130px; color: #8b949e; }
    .form-row input[type="text"],
    .form-row input[type="password"],
    .form-row input[type="number"],
    .form-row select { padding: 6px 10px; background: #0d1117; border: 1px solid #30363d; border-radius: 4px; color: #c9d1d9; font-size: 13px; width: 200px; }
    .form-row input[type="checkbox"] { accent-color: #58a6ff; width: auto; }
    .form-row select { width: 218px; }
    .form-actions { display: flex; gap: 8px; margin-top: 14px; }

    /* General settings */
    .setting-row { display: flex; align-items: center; gap: 12px; margin-bottom: 12px; }
    .setting-row label { font-size: 13px; min-width: 280px; }
    .setting-input { padding: 6px 10px; background: #0d1117; border: 1px solid #30363d; border-radius: 4px; color: #c9d1d9; font-size: 13px; width: 150px; }

    .empty { padding: 24px; color: #484f58; text-align: center; }
  `],
})
export class SettingsViewComponent implements OnInit {
  tab = 'servers';

  // Servers
  servers = signal<ServerConfig[]>([]);
  editingServer: ServerConfig | null = null;
  editingServerId: string | null = null; // non-null when editing existing

  // Categories
  categories = signal<CategoryConfig[]>([]);
  editingCategory: CategoryConfig | null = null;
  editingCategoryOriginalName: string | null = null; // non-null when editing existing

  // General
  speedLimit = 0;
  maxActiveDownloads = 3;
  historyRetention: number | null = null;

  constructor(private api: ApiService, private snack: MatSnackBar) {}

  ngOnInit(): void {
    this.loadServers();
    this.loadCategories();
    this.loadGeneralSettings();
  }

  // ======================== SERVERS ========================

  loadServers(): void {
    this.api.get<ServerConfig[]>('/config/servers').subscribe({
      next: r => this.servers.set(r),
      error: () => {},
    });
  }

  addServer(): void {
    this.editingServer = emptyServer();
    this.editingServerId = null;
  }

  editServer(s: ServerConfig): void {
    this.editingServer = { ...s };
    this.editingServerId = s.id;
  }

  cancelServerEdit(): void {
    this.editingServer = null;
    this.editingServerId = null;
  }

  saveServer(): void {
    if (!this.editingServer) return;
    const server = { ...this.editingServer };
    if (!server.username) server.username = null;
    if (!server.password) server.password = null;

    if (this.editingServerId) {
      // Edit existing
      this.api.put(`/config/servers/${this.editingServerId}`, server).subscribe({
        next: () => {
          this.snack.open('Server updated', 'Close', { duration: 2000 });
          this.cancelServerEdit();
          this.loadServers();
        },
        error: () => this.snack.open('Failed to update server', 'Close', { duration: 3000 }),
      });
    } else {
      // Add new
      server.id = '';
      this.api.post('/config/servers', server).subscribe({
        next: () => {
          this.snack.open('Server added', 'Close', { duration: 2000 });
          this.cancelServerEdit();
          this.loadServers();
        },
        error: () => this.snack.open('Failed to add server', 'Close', { duration: 3000 }),
      });
    }
  }

  testServer(id: string): void {
    this.api.post<{ success: boolean; message: string }>(`/config/servers/${id}/test`).subscribe({
      next: r => this.snack.open(r.message, 'Close', { duration: 3000 }),
      error: () => this.snack.open('Test failed', 'Close', { duration: 3000 }),
    });
  }

  deleteServer(id: string): void {
    this.api.delete(`/config/servers/${id}`).subscribe({
      next: () => { this.loadServers(); this.snack.open('Server removed', 'Close', { duration: 2000 }); },
      error: () => this.snack.open('Failed to delete server', 'Close', { duration: 3000 }),
    });
  }

  // ======================== CATEGORIES ========================

  loadCategories(): void {
    this.api.get<CategoryConfig[]>('/config/categories').subscribe({
      next: r => this.categories.set(r),
      error: () => {},
    });
  }

  addCategory(): void {
    this.editingCategory = emptyCategory();
    this.editingCategoryOriginalName = null;
  }

  editCategory(c: CategoryConfig): void {
    this.editingCategory = { ...c };
    this.editingCategoryOriginalName = c.name;
  }

  cancelCategoryEdit(): void {
    this.editingCategory = null;
    this.editingCategoryOriginalName = null;
  }

  saveCategory(): void {
    if (!this.editingCategory) return;
    const cat = { ...this.editingCategory };
    if (!cat.output_dir) cat.output_dir = null;

    if (this.editingCategoryOriginalName) {
      // Edit existing
      const encoded = encodeURIComponent(this.editingCategoryOriginalName);
      this.api.put(`/config/categories/${encoded}`, cat).subscribe({
        next: () => {
          this.snack.open('Category updated', 'Close', { duration: 2000 });
          this.cancelCategoryEdit();
          this.loadCategories();
        },
        error: () => this.snack.open('Failed to update category', 'Close', { duration: 3000 }),
      });
    } else {
      // Add new
      this.api.post('/config/categories', cat).subscribe({
        next: () => {
          this.snack.open('Category added', 'Close', { duration: 2000 });
          this.cancelCategoryEdit();
          this.loadCategories();
        },
        error: () => this.snack.open('Failed to add category', 'Close', { duration: 3000 }),
      });
    }
  }

  deleteCategory(name: string): void {
    const encoded = encodeURIComponent(name);
    this.api.delete(`/config/categories/${encoded}`).subscribe({
      next: () => { this.loadCategories(); this.snack.open('Category removed', 'Close', { duration: 2000 }); },
      error: () => this.snack.open('Failed to delete category', 'Close', { duration: 3000 }),
    });
  }

  ppLabel(level: number): string {
    switch (level) {
      case 0: return 'None';
      case 1: return 'Repair';
      case 2: return 'Unpack';
      case 3: return 'Repair + Unpack';
      default: return 'Unknown';
    }
  }

  // ======================== GENERAL ========================

  loadGeneralSettings(): void {
    this.api.get<{ speed_limit_bps: number }>('/config/speed-limit').subscribe({
      next: r => this.speedLimit = r.speed_limit_bps,
      error: () => {},
    });
    this.api.get<{ max_active_downloads: number }>('/config/max-active-downloads').subscribe({
      next: r => this.maxActiveDownloads = r.max_active_downloads,
      error: () => {},
    });
    this.api.get<{ retention: number | null }>('/config/history-retention').subscribe({
      next: r => this.historyRetention = r.retention,
      error: () => {},
    });
  }

  saveSpeedLimit(): void {
    this.api.put('/config/speed-limit', { speed_limit_bps: this.speedLimit }).subscribe({
      next: () => this.snack.open('Speed limit saved', 'Close', { duration: 2000 }),
      error: () => this.snack.open('Failed to save speed limit', 'Close', { duration: 3000 }),
    });
  }

  saveMaxActive(): void {
    this.api.put('/config/max-active-downloads', { max_active_downloads: this.maxActiveDownloads }).subscribe({
      next: () => this.snack.open('Max downloads saved', 'Close', { duration: 2000 }),
      error: () => this.snack.open('Failed to save max downloads', 'Close', { duration: 3000 }),
    });
  }

  saveRetention(): void {
    this.api.put('/config/history-retention', { retention: this.historyRetention }).subscribe({
      next: () => this.snack.open('History retention saved', 'Close', { duration: 2000 }),
      error: () => this.snack.open('Failed to save retention', 'Close', { duration: 3000 }),
    });
  }
}
