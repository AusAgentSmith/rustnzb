import { Injectable } from '@angular/core';
import { HttpClient, HttpHeaders } from '@angular/common/http';
import { Observable } from 'rxjs';

@Injectable({ providedIn: 'root' })
export class ApiService {
  private baseUrl = '/api';

  constructor(private http: HttpClient) {}

  private headers(): HttpHeaders {
    const token = localStorage.getItem('access_token');
    return token ? new HttpHeaders({ Authorization: `Bearer ${token}` }) : new HttpHeaders();
  }

  get<T>(path: string, params?: Record<string, string>): Observable<T> {
    return this.http.get<T>(`${this.baseUrl}${path}`, { headers: this.headers(), params });
  }

  post<T>(path: string, body?: unknown): Observable<T> {
    return this.http.post<T>(`${this.baseUrl}${path}`, body, { headers: this.headers() });
  }

  put<T>(path: string, body: unknown): Observable<T> {
    return this.http.put<T>(`${this.baseUrl}${path}`, body, { headers: this.headers() });
  }

  delete<T>(path: string): Observable<T> {
    return this.http.delete<T>(`${this.baseUrl}${path}`, { headers: this.headers() });
  }

  postForm<T>(path: string, body: FormData): Observable<T> {
    const token = localStorage.getItem('access_token');
    const headers = token ? new HttpHeaders({ Authorization: `Bearer ${token}` }) : new HttpHeaders();
    return this.http.post<T>(`${this.baseUrl}${path}`, body, { headers });
  }
}
