import { Component, OnInit, signal, computed } from '@angular/core';
import { CommonModule } from '@angular/common';
import { FormsModule } from '@angular/forms';
import { MatIconModule } from '@angular/material/icon';
import { MatButtonModule } from '@angular/material/button';
import { MatProgressBarModule } from '@angular/material/progress-bar';
import { MatProgressSpinnerModule } from '@angular/material/progress-spinner';
import { MatSnackBar, MatSnackBarModule } from '@angular/material/snack-bar';
import { MatTooltipModule } from '@angular/material/tooltip';
import { MatDialog, MatDialogModule } from '@angular/material/dialog';
import { GroupService } from '../../core/services/group.service';
import { GroupRow, HeaderRow } from '../../core/models/group.model';
import { GroupBrowserDialogComponent } from './group-browser-dialog.component';

@Component({
  selector: 'app-groups-view',
  standalone: true,
  imports: [
    CommonModule, FormsModule,
    MatIconModule, MatButtonModule, MatProgressBarModule, MatProgressSpinnerModule,
    MatSnackBarModule, MatTooltipModule, MatDialogModule,
  ],
  template: `
    <div class="groups-layout">
      <!-- Sidebar -->
      <div class="sidebar">
        <div class="sidebar-header">
          <span>Groups</span>
          <button class="icon-btn" matTooltip="Browse & subscribe" (click)="openBrowseDialog()">＋</button>
        </div>
        <div class="sidebar-search">
          <input [(ngModel)]="groupFilter" placeholder="Filter groups..." />
        </div>
        <div class="sidebar-list">
          @for (g of filteredGroups(); track g.id) {
            <div class="group-item" [class.selected]="selectedGroup()?.id === g.id" (click)="selectGroup(g)">
              <span class="group-name">{{ g.name }}</span>
              @if (g.unread_count > 0) { <span class="unread-badge">{{ g.unread_count }}</span> }
            </div>
          }
          @if (groups().length === 0) {
            <div class="sidebar-empty">
              <button class="link-btn" (click)="openBrowseDialog()">Browse &amp; subscribe</button>
            </div>
          }
        </div>
      </div>

      <!-- Right panel -->
      <div class="right-panel">
        @if (!selectedGroup()) {
          <div class="empty-state"><div style="font-size:48px;opacity:0.3;margin-bottom:12px">📡</div><p>Select a group to browse</p></div>
        } @else {
          <!-- Toolbar -->
          <div class="panel-toolbar">
            <span class="panel-title">{{ selectedGroup()!.name }}</span>
            <span class="panel-stats">{{ headerTotal() }} articles
              @if (newAvailable() > 0) { · <span class="new-count">{{ newAvailable() }} new</span> }
            </span>
            <span class="spacer"></span>
            <div class="search-box">
              <span class="search-icon">🔍</span>
              <input [(ngModel)]="searchQuery" (keyup.enter)="searchHeaders()" placeholder="Search headers..." />
            </div>
            <button class="tool-btn" matTooltip="Fetch headers" (click)="fetchHeaders()" [disabled]="fetching()">↻</button>
            <button class="tool-btn" matTooltip="Mark all read" (click)="markAllRead()">✓</button>
          </div>

          @if (fetching()) { <mat-progress-bar mode="indeterminate" class="fetch-bar"></mat-progress-bar> }

          <!-- Header table -->
          <div class="header-list">
            <div class="header-table-head">
              <div class="col-cb"><input type="checkbox" [checked]="allSelected()" (change)="toggleSelectAll()" /></div>
              <div class="col-subject">Subject</div>
              <div class="col-author">Author</div>
              <div class="col-size">Size</div>
              <div class="col-date">Date</div>
            </div>
            <div class="header-table-body">
              @for (h of headers(); track h.id) {
                <div class="header-row" [class.selected]="previewHeader()?.id === h.id"
                     [class.unread]="!h.read" [class.read-row]="h.read" (click)="selectArticle(h)">
                  <div class="col-cb" (click)="$event.stopPropagation()">
                    <input type="checkbox" [checked]="isSelected(h.message_id)" (change)="toggleSelect(h.message_id)" />
                  </div>
                  <div class="col-subject">{{ h.subject }}</div>
                  <div class="col-author">{{ h.author }}</div>
                  <div class="col-size">{{ formatBytes(h.bytes) }}</div>
                  <div class="col-date">{{ h.date }}</div>
                </div>
              }
              @if (headers().length === 0 && !fetching()) {
                <div class="list-empty">No headers. @if (newAvailable() > 0) { Click ↻ to fetch. }</div>
              }
            </div>
            @if (headerTotal() > headers().length) {
              <div class="load-more"><button class="tool-btn" (click)="loadMore()">Load more...</button></div>
            }
          </div>

          <!-- Download bar -->
          @if (selectedIds().length > 0) {
            <div class="download-bar">
              <span class="sel-count">{{ selectedIds().length }} selected · {{ formatBytes(selectedBytes()) }}</span>
              <span class="spacer"></span>
              <button class="download-btn" (click)="downloadSelected()">↓ Download Selected</button>
            </div>
          }

          <!-- Preview -->
          @if (previewHeader()) {
            <div class="preview-pane">
              @if (articleLoading()) {
                <div class="preview-loading"><mat-spinner diameter="28"></mat-spinner></div>
              } @else {
                <div class="preview-head">
                  <div><strong>From:</strong> {{ previewHeader()!.author }} · <strong>Subject:</strong> {{ previewHeader()!.subject }}</div>
                  <div><strong>Message-ID:</strong> <span class="msgid">{{ previewHeader()!.message_id }}</span></div>
                </div>
                <div class="preview-body"><pre>{{ articleBody() || '(empty)' }}</pre></div>
              }
            </div>
          }
        }
      </div>
    </div>
  `,
  styles: [`
    :host { display: flex; height: 100%; overflow: hidden; }
    .groups-layout { display: flex; flex: 1; overflow: hidden; }
    .sidebar { width: 240px; min-width: 180px; border-right: 1px solid #21262d; display: flex; flex-direction: column; background: #0d1117; }
    .sidebar-header { display: flex; align-items: center; justify-content: space-between; padding: 8px 12px; font-size: 11px; font-weight: 600; text-transform: uppercase; color: #8b949e; border-bottom: 1px solid #21262d; }
    .icon-btn { background: none; border: none; color: #8b949e; cursor: pointer; font-size: 16px; padding: 2px 4px; border-radius: 3px; }
    .icon-btn:hover { background: #21262d; color: #c9d1d9; }
    .sidebar-search { padding: 6px 8px; border-bottom: 1px solid #21262d; }
    .sidebar-search input { width: 100%; padding: 5px 8px; background: #161b22; border: 1px solid #30363d; border-radius: 4px; color: #c9d1d9; font-size: 12px; outline: none; }
    .sidebar-search input:focus { border-color: #58a6ff; }
    .sidebar-list { flex: 1; overflow-y: auto; }
    .group-item { display: flex; align-items: center; justify-content: space-between; padding: 7px 12px; cursor: pointer; border-left: 3px solid transparent; font-size: 12px; }
    .group-item:hover { background: #161b22; }
    .group-item.selected { background: rgba(56,139,253,0.1); border-left-color: #58a6ff; }
    .group-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
    .unread-badge { background: #388bfd; color: white; font-size: 10px; font-weight: 700; padding: 1px 5px; border-radius: 8px; }
    .sidebar-empty { padding: 16px; text-align: center; font-size: 12px; color: #484f58; }
    .link-btn { background: none; border: none; color: #58a6ff; cursor: pointer; font-size: 12px; }
    .right-panel { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
    .empty-state { display: flex; flex-direction: column; align-items: center; justify-content: center; height: 100%; color: #484f58; }
    .panel-toolbar { display: flex; align-items: center; gap: 8px; padding: 6px 12px; background: #0d1117; border-bottom: 1px solid #21262d; flex-shrink: 0; }
    .panel-title { font-weight: 600; font-size: 14px; }
    .panel-stats { font-size: 11px; color: #8b949e; margin-left: 8px; }
    .new-count { color: #3fb950; }
    .spacer { flex: 1; }
    .search-box { display: flex; align-items: center; gap: 4px; background: #161b22; border: 1px solid #30363d; border-radius: 4px; padding: 3px 8px; }
    .search-icon { font-size: 12px; color: #484f58; }
    .search-box input { background: none; border: none; color: #c9d1d9; font-size: 12px; outline: none; width: 160px; }
    .tool-btn { padding: 3px 8px; border-radius: 4px; border: 1px solid #30363d; background: #21262d; color: #c9d1d9; cursor: pointer; font-size: 12px; }
    .tool-btn:hover { background: #30363d; }
    .tool-btn:disabled { opacity: 0.4; }
    .fetch-bar { flex-shrink: 0; }
    .header-list { flex: 1; display: flex; flex-direction: column; overflow: hidden; min-height: 150px; }
    .header-table-head { display: flex; padding: 4px 0; font-size: 11px; font-weight: 600; color: #8b949e; background: #161b22; border-bottom: 1px solid #30363d; flex-shrink: 0; }
    .header-table-body { flex: 1; overflow-y: auto; }
    .header-row { display: flex; align-items: center; padding: 4px 0; cursor: pointer; border-bottom: 1px solid #21262d; font-size: 12px; }
    .header-row:hover { background: #161b22; }
    .header-row.selected { background: rgba(56,139,253,0.1); }
    .header-row.unread { font-weight: 600; }
    .header-row.read-row { color: #8b949e; }
    .col-cb { width: 36px; text-align: center; flex-shrink: 0; }
    .col-subject { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; padding: 0 8px; }
    .col-author { width: 160px; flex-shrink: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: #8b949e; padding: 0 4px; }
    .col-size { width: 70px; flex-shrink: 0; text-align: right; color: #8b949e; padding: 0 8px; }
    .col-date { width: 140px; flex-shrink: 0; color: #8b949e; padding: 0 8px; }
    .list-empty { padding: 24px; text-align: center; color: #484f58; font-size: 12px; }
    .load-more { text-align: center; padding: 6px; border-top: 1px solid #21262d; flex-shrink: 0; }
    input[type="checkbox"] { accent-color: #58a6ff; }
    .download-bar { display: flex; align-items: center; gap: 8px; padding: 8px 12px; background: #161b22; border-top: 1px solid #30363d; flex-shrink: 0; }
    .sel-count { font-size: 12px; color: #8b949e; }
    .download-btn { padding: 6px 16px; border-radius: 4px; border: none; background: #238636; color: white; cursor: pointer; font-size: 13px; font-weight: 600; }
    .download-btn:hover { background: #2ea043; }
    .preview-pane { display: flex; flex-direction: column; border-top: 3px solid #30363d; height: 200px; flex-shrink: 0; overflow: hidden; }
    .preview-head { padding: 6px 12px; background: #161b22; border-bottom: 1px solid #21262d; font-size: 11px; line-height: 1.6; }
    .preview-head strong { color: #8b949e; }
    .msgid { color: #484f58; font-size: 10px; }
    .preview-body { flex: 1; padding: 10px 12px; overflow-y: auto; }
    .preview-body pre { margin: 0; white-space: pre-wrap; word-wrap: break-word; font-family: 'JetBrains Mono', Consolas, monospace; font-size: 12px; line-height: 1.5; }
    .preview-loading { display: flex; justify-content: center; padding: 24px; }
  `],
})
export class GroupsViewComponent implements OnInit {
  groups = signal<GroupRow[]>([]);
  selectedGroup = signal<GroupRow | null>(null);
  groupFilter = '';
  headers = signal<HeaderRow[]>([]);
  headerTotal = signal(0);
  searchQuery = '';
  offset = 0;
  pageSize = 100;
  selectedIds = signal<string[]>([]);
  newAvailable = signal(0);
  fetching = signal(false);
  previewHeader = signal<HeaderRow | null>(null);
  articleBody = signal<string | null>(null);
  articleLoading = signal(false);

  filteredGroups = computed(() => {
    const f = this.groupFilter.toLowerCase();
    return f ? this.groups().filter(g => g.name.toLowerCase().includes(f)) : this.groups();
  });
  allSelected = computed(() => {
    const ids = this.selectedIds(), hdrs = this.headers();
    return hdrs.length > 0 && hdrs.every(h => ids.includes(h.message_id));
  });
  selectedBytes = computed(() => {
    const ids = new Set(this.selectedIds());
    return this.headers().filter(h => ids.has(h.message_id)).reduce((s, h) => s + h.bytes, 0);
  });

  constructor(private svc: GroupService, private snack: MatSnackBar, private dialog: MatDialog) {}

  ngOnInit(): void { this.loadGroups(); }

  loadGroups(): void { this.svc.list({ subscribed: true, limit: 500 }).subscribe(r => this.groups.set(r.groups)); }

  selectGroup(g: GroupRow): void {
    this.selectedGroup.set(g);
    this.offset = 0; this.searchQuery = ''; this.selectedIds.set([]);
    this.previewHeader.set(null); this.articleBody.set(null);
    this.loadHeaders(); this.loadStatus();
  }

  loadHeaders(): void {
    const g = this.selectedGroup(); if (!g) return;
    this.svc.listHeaders(g.id, { search: this.searchQuery || undefined, limit: this.pageSize, offset: this.offset })
      .subscribe(r => { this.headers.set(r.headers); this.headerTotal.set(r.total); });
  }

  loadStatus(): void {
    const g = this.selectedGroup(); if (!g) return;
    this.svc.getStatus(g.id).subscribe(s => this.newAvailable.set(s.new_available));
  }

  searchHeaders(): void { this.offset = 0; this.loadHeaders(); }

  loadMore(): void {
    this.offset += this.pageSize;
    const g = this.selectedGroup(); if (!g) return;
    this.svc.listHeaders(g.id, { search: this.searchQuery || undefined, limit: this.pageSize, offset: this.offset })
      .subscribe(r => this.headers.set([...this.headers(), ...r.headers]));
  }

  fetchHeaders(): void {
    const g = this.selectedGroup(); if (!g) return;
    this.fetching.set(true);
    this.svc.fetchHeaders(g.id).subscribe({ next: () => this.snack.open('Fetching headers...', 'Close', { duration: 2000 }), error: () => this.fetching.set(false) });
    const poll = setInterval(() => { this.loadHeaders(); this.loadStatus(); this.loadGroups(); if (this.newAvailable() <= 0) { this.fetching.set(false); clearInterval(poll); } }, 3000);
    setTimeout(() => { clearInterval(poll); this.fetching.set(false); }, 120000);
  }

  markAllRead(): void {
    const g = this.selectedGroup(); if (!g) return;
    this.svc.markAllRead(g.id).subscribe(() => { this.loadHeaders(); this.loadGroups(); this.snack.open('All marked read', 'Close', { duration: 2000 }); });
  }

  toggleSelect(mid: string): void {
    const ids = this.selectedIds();
    this.selectedIds.set(ids.includes(mid) ? ids.filter(i => i !== mid) : [...ids, mid]);
  }
  isSelected(mid: string): boolean { return this.selectedIds().includes(mid); }
  toggleSelectAll(): void {
    this.selectedIds.set(this.allSelected() ? [] : this.headers().map(h => h.message_id));
  }

  selectArticle(h: HeaderRow): void {
    this.previewHeader.set(h); this.articleLoading.set(true); this.articleBody.set(null);
    this.svc.getArticle(h.message_id).subscribe({
      next: r => { this.articleBody.set(r.body); this.articleLoading.set(false); if (!h.read) { h.read = true; this.headers.set([...this.headers()]); } },
      error: () => { this.articleBody.set('(Failed to load)'); this.articleLoading.set(false); },
    });
  }

  downloadSelected(): void {
    const g = this.selectedGroup(); if (!g || !this.selectedIds().length) return;
    this.svc.downloadSelected(g.id, this.selectedIds()).subscribe({
      next: r => { this.snack.open(r.message, 'Close', { duration: 3000 }); this.selectedIds.set([]); },
      error: e => this.snack.open('Download failed', 'Close', { duration: 5000 }),
    });
  }

  openBrowseDialog(): void {
    this.dialog.open(GroupBrowserDialogComponent, { width: '700px', maxHeight: '80vh' }).afterClosed().subscribe(() => this.loadGroups());
  }

  formatBytes(b: number): string {
    if (b === 0) return '0 B';
    const k = 1024, s = ['B', 'KB', 'MB', 'GB', 'TB'], i = Math.floor(Math.log(b) / Math.log(k));
    return parseFloat((b / Math.pow(k, i)).toFixed(1)) + ' ' + s[i];
  }
}
