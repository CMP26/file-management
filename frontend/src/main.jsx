import React, { useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  Activity,
  ArrowLeft,
  BookOpen,
  Bot,
  Check,
  CheckCircle2,
  ChevronRight,
  CircleHelp,
  Clock3,
  CloudDownload,
  FileQuestion,
  Film,
  FolderOpen,
  Gauge,
  GraduationCap,
  Library,
  ListVideo,
  LoaderCircle,
  Menu,
  MessageSquare,
  Play,
  Plus,
  RefreshCw,
  Search,
  Send,
  Server,
  Sparkles,
  Trash2,
  Upload,
  UserRound,
  Video,
  X
} from "lucide-react";
import "./styles.css";

const STUDENTS = [
  { id: "11111111-1111-4111-8111-111111111111", name: "Ahmed" },
  { id: "22222222-2222-4222-8222-222222222222", name: "Mariam" },
  { id: "33333333-3333-4333-8333-333333333333", name: "Omar" }
];
const API_BASE = window.location.origin === "null" ? "http://localhost:8080" : window.location.origin;
const TERMINAL_VIDEO_STATUSES = new Set(["ready", "failed"]);

async function api(path, options = {}) {
  let response;
  try {
    response = await fetch(`${API_BASE}${path}`, options);
  } catch {
    throw new Error("The backend could not be reached.");
  }

  const contentType = response.headers.get("content-type") || "";
  const body = contentType.includes("application/json") ? await response.json() : await response.text();
  if (!response.ok) {
    throw new Error(body?.message || `${response.status} ${response.statusText}`);
  }
  return body;
}

function classNames(...values) {
  return values.filter(Boolean).join(" ");
}

function shortId(value) {
  return value ? `${value.slice(0, 7)}...${value.slice(-4)}` : "";
}

function formatDate(value) {
  if (!value) return "";
  return new Intl.DateTimeFormat(undefined, {
    month: "short",
    day: "numeric",
    hour: "numeric",
    minute: "2-digit"
  }).format(new Date(value));
}

function formatDuration(seconds) {
  if (seconds == null) return "--";
  const total = Math.max(0, Math.floor(seconds));
  const hours = Math.floor(total / 3600);
  const minutes = Math.floor((total % 3600) / 60);
  const remainder = total % 60;
  return hours
    ? `${hours}:${String(minutes).padStart(2, "0")}:${String(remainder).padStart(2, "0")}`
    : `${minutes}:${String(remainder).padStart(2, "0")}`;
}

function titleCase(value) {
  return String(value || "")
    .replaceAll("_", " ")
    .replace(/\b\w/g, (character) => character.toUpperCase());
}

function App() {
  const [view, setView] = useState("overview");
  const [mobileNav, setMobileNav] = useState(false);
  const [courses, setCourses] = useState([]);
  const [videos, setVideos] = useState([]);
  const [health, setHealth] = useState("checking");
  const [llm, setLlm] = useState(null);
  const [selectedVideoId, setSelectedVideoId] = useState(null);
  const [uploadOpen, setUploadOpen] = useState(false);
  const [courseOpen, setCourseOpen] = useState(false);
  const [toast, setToast] = useState(null);
  const [loading, setLoading] = useState(true);
  const [studentId, setStudentId] = useState(
    () => window.localStorage.getItem("nexalearnStudentId") || STUDENTS[0].id
  );

  const selectedVideo = videos.find((video) => video.id === selectedVideoId) || null;
  const selectedStudent = STUDENTS.find((student) => student.id === studentId) || STUDENTS[0];

  const notify = (message, tone = "neutral") => {
    setToast({ message, tone });
    window.setTimeout(() => setToast(null), 3600);
  };

  const loadCore = async (quiet = false) => {
    try {
      const [courseData, videoData] = await Promise.all([
        api("/api/courses"),
        api("/api/videos")
      ]);
      setCourses(courseData.courses || []);
      setVideos(videoData.videos || []);
      if (!quiet && !selectedVideoId && videoData.videos?.length) {
        setSelectedVideoId(videoData.videos[0].id);
      }
    } catch (error) {
      notify(error.message, "danger");
    } finally {
      setLoading(false);
    }
  };

  const checkServices = async () => {
    try {
      const value = await api("/healthz");
      setHealth(value === "ok" ? "online" : value);
    } catch {
      setHealth("offline");
    }
    try {
      setLlm(await api("/api/llm/status"));
    } catch {
      setLlm({ reachable: false });
    }
  };

  useEffect(() => {
    loadCore();
    checkServices();
  }, []);

  useEffect(() => {
    const active = videos.some((video) => !TERMINAL_VIDEO_STATUSES.has(video.status));
    if (!active) return undefined;
    const timer = window.setInterval(() => loadCore(true), 5000);
    return () => window.clearInterval(timer);
  }, [videos]);

  const navigate = (nextView) => {
    setView(nextView);
    setMobileNav(false);
  };

  const switchStudent = (nextStudentId) => {
    setStudentId(nextStudentId);
    window.localStorage.setItem("nexalearnStudentId", nextStudentId);
  };

  const openStudyRoom = (videoId) => {
    setSelectedVideoId(videoId);
    navigate("study");
  };

  const deleteLesson = async (video) => {
    if (!window.confirm(`Delete "${video.title}" and all generated data?`)) return;
    try {
      await api(`/api/videos/${video.id}`, { method: "DELETE" });
      if (selectedVideoId === video.id) {
        setSelectedVideoId(videos.find((item) => item.id !== video.id)?.id || null);
      }
      await loadCore(true);
      notify("Lesson deleted.", "success");
    } catch (error) {
      notify(error.message, "danger");
    }
  };

  const deleteCourse = async (course) => {
    if (!window.confirm(`Delete course "${course.title}"?`)) return;
    try {
      await api(`/api/courses/${course.id}`, { method: "DELETE" });
      await loadCore(true);
      notify("Course deleted.", "success");
    } catch (error) {
      notify(error.message, "danger");
    }
  };

  const stats = useMemo(() => ({
    courses: courses.length,
    lessons: videos.length,
    ready: videos.filter((video) => video.status === "ready").length,
    questions: videos.reduce((sum, video) => sum + Number(video.question_count || 0), 0)
  }), [courses, videos]);

  const navItems = [
    { id: "overview", label: "Overview", icon: Gauge },
    { id: "courses", label: "Courses", icon: GraduationCap },
    { id: "library", label: "Library", icon: Library },
    { id: "question-bank", label: "Question Bank", icon: FileQuestion },
    { id: "study", label: "Study Room", icon: Sparkles }
  ];

  return (
    <div className="app-shell">
      <aside className={classNames("sidebar", mobileNav && "sidebar-open")}>
        <div className="brand-lockup">
          <div className="brand-mark"><BookOpen size={19} /></div>
          <div>
            <strong>NexaLearn</strong>
            <span>Learning workspace</span>
          </div>
          <button className="icon-button mobile-only" onClick={() => setMobileNav(false)} aria-label="Close navigation">
            <X size={18} />
          </button>
        </div>

        <nav className="main-nav">
          {navItems.map(({ id, label, icon: Icon }) => (
            <button
              key={id}
              className={classNames("nav-item", view === id && "active")}
              onClick={() => navigate(id)}
            >
              <Icon size={18} />
              <span>{label}</span>
            </button>
          ))}
        </nav>

        <div className="sidebar-status">
          <div className="status-row">
            <span className={classNames("service-dot", health === "online" ? "good" : "bad")} />
            <span>Backend</span>
            <strong>{health}</strong>
          </div>
          <div className="status-row">
            <span className={classNames("service-dot", llm?.reachable ? "good" : "bad")} />
            <span>Gemma</span>
            <strong>{llm?.reachable ? "ready" : "offline"}</strong>
          </div>
        </div>

        <div className="profile-strip">
          <div className="avatar"><UserRound size={17} /></div>
          <div>
            <select className="student-switcher" value={studentId} onChange={(event) => switchStudent(event.target.value)}>
              {STUDENTS.map((student) => <option key={student.id} value={student.id}>{student.name}</option>)}
            </select>
            <span title={studentId}>{shortId(studentId)}</span>
          </div>
        </div>
      </aside>

      <div className="main-area">
        <header className="topbar">
          <button className="icon-button mobile-only" onClick={() => setMobileNav(true)} aria-label="Open navigation">
            <Menu size={20} />
          </button>
          <div className="topbar-title">
            <span>{navItems.find((item) => item.id === view)?.label}</span>
            {selectedVideo && view === "study" && <small>{selectedVideo.title}</small>}
          </div>
          <div className="topbar-actions">
            <button className="icon-button" onClick={() => { loadCore(true); checkServices(); }} title="Refresh">
              <RefreshCw size={18} />
            </button>
            <button className="button secondary" onClick={() => setCourseOpen(true)}>
              <Plus size={17} /><span>Course</span>
            </button>
            <button className="button primary" onClick={() => setUploadOpen(true)}>
              <Upload size={17} /><span>Add lesson</span>
            </button>
          </div>
        </header>

        <main className="content">
          {loading ? (
            <LoadingState label="Loading workspace" />
          ) : (
            <>
              {view === "overview" && (
                <Overview
                  stats={stats}
                  courses={courses}
                  videos={videos}
                  openStudyRoom={openStudyRoom}
                  navigate={navigate}
                  openUpload={() => setUploadOpen(true)}
                />
              )}
              {view === "courses" && (
                <CoursesView
                  courses={courses}
                  videos={videos}
                  createCourse={() => setCourseOpen(true)}
                  openStudyRoom={openStudyRoom}
                  deleteCourse={deleteCourse}
                  deleteLesson={deleteLesson}
                />
              )}
              {view === "library" && (
                <LibraryView
                  videos={videos}
                  courses={courses}
                  openStudyRoom={openStudyRoom}
                  openUpload={() => setUploadOpen(true)}
                  refresh={() => loadCore(true)}
                  deleteLesson={deleteLesson}
                />
              )}
              {view === "question-bank" && (
                <QuestionBankView courses={courses} videos={videos} notify={notify} />
              )}
              {view === "study" && (
                <StudyRoom
                  key={`${selectedVideoId || "none"}:${studentId}`}
                  videos={videos}
                  selectedVideoId={selectedVideoId}
                  setSelectedVideoId={setSelectedVideoId}
                  refreshVideos={() => loadCore(true)}
                  notify={notify}
                  userId={studentId}
                  studentName={selectedStudent.name}
                />
              )}
            </>
          )}
        </main>
      </div>

      {mobileNav && <button className="sidebar-scrim" onClick={() => setMobileNav(false)} aria-label="Close navigation" />}
      {uploadOpen && (
        <UploadDialog
          courses={courses}
          close={() => setUploadOpen(false)}
          complete={(videoId) => {
            setUploadOpen(false);
            loadCore(true);
            openStudyRoom(videoId);
          }}
          notify={notify}
        />
      )}
      {courseOpen && (
        <CourseDialog
          close={() => setCourseOpen(false)}
          complete={() => {
            setCourseOpen(false);
            loadCore(true);
          }}
          notify={notify}
        />
      )}
      {toast && <div className={classNames("toast", toast.tone)}>{toast.message}</div>}
    </div>
  );
}

function Overview({ stats, courses, videos, openStudyRoom, navigate, openUpload }) {
  const recent = videos.slice(0, 5);
  const processing = videos.filter((video) => !TERMINAL_VIDEO_STATUSES.has(video.status));

  return (
    <div className="page-stack">
      <section className="page-heading">
        <div>
          <p className="eyebrow">Wednesday workspace</p>
          <h1>Keep learning moving.</h1>
          <p>Manage course content, study transcripts, ask grounded questions, and assess understanding.</p>
        </div>
        <button className="button primary" onClick={openUpload}><Upload size={17} /> Upload lesson</button>
      </section>

      <section className="stat-grid">
        <StatItem label="Courses" value={stats.courses} icon={GraduationCap} tone="green" />
        <StatItem label="Lessons" value={stats.lessons} icon={Film} tone="blue" />
        <StatItem label="Ready to study" value={stats.ready} icon={CheckCircle2} tone="amber" />
        <StatItem label="Question bank" value={stats.questions} icon={FileQuestion} tone="red" />
      </section>

      {processing.length > 0 && (
        <section className="notice-band">
          <LoaderCircle className="spin" size={19} />
          <div>
            <strong>{processing.length} lesson{processing.length === 1 ? "" : "s"} processing</strong>
            <span>{processing.map((video) => `${video.title}: ${titleCase(video.status)}`).join(" · ")}</span>
          </div>
          <button className="text-button" onClick={() => navigate("library")}>View library <ChevronRight size={16} /></button>
        </section>
      )}

      <div className="dashboard-grid">
        <section className="surface">
          <div className="section-header">
            <div><h2>Recent lessons</h2><p>Continue from your latest uploads.</p></div>
            <button className="text-button" onClick={() => navigate("library")}>All lessons <ChevronRight size={16} /></button>
          </div>
          <div className="lesson-table">
            {recent.length ? recent.map((video) => (
              <button className="lesson-row" key={video.id} onClick={() => openStudyRoom(video.id)}>
                <div className="lesson-symbol"><Play size={17} /></div>
                <div className="lesson-copy">
                  <strong>{video.title}</strong>
                  <span>{video.course_title} · {formatDate(video.created_at)}</span>
                </div>
                <StatusPill status={video.status} />
                <div className="lesson-count">{video.question_count} questions</div>
                <ChevronRight size={17} />
              </button>
            )) : <EmptyState icon={Film} title="No lessons yet" body="Upload your first lesson to begin." />}
          </div>
        </section>

        <section className="surface">
          <div className="section-header">
            <div><h2>Courses</h2><p>Content coverage at a glance.</p></div>
            <button className="text-button" onClick={() => navigate("courses")}>Manage <ChevronRight size={16} /></button>
          </div>
          <div className="course-summary-list">
            {courses.slice(0, 5).map((course) => (
              <div className="course-summary" key={course.id}>
                <div className="course-letter">{course.title.slice(0, 1).toUpperCase()}</div>
                <div>
                  <strong>{course.title}</strong>
                  <span>{course.video_count} lessons</span>
                </div>
                <b>{course.question_count}</b>
                <small>questions</small>
              </div>
            ))}
            {!courses.length && <EmptyState icon={GraduationCap} title="No courses" body="Create a course before uploading lessons." />}
          </div>
        </section>
      </div>
    </div>
  );
}

function StatItem({ label, value, icon: Icon, tone }) {
  return (
    <div className="stat-item">
      <div className={classNames("stat-icon", tone)}><Icon size={19} /></div>
      <span>{label}</span>
      <strong>{value}</strong>
    </div>
  );
}

function CoursesView({ courses, videos, createCourse, openStudyRoom, deleteCourse, deleteLesson }) {
  const [selectedCourseId, setSelectedCourseId] = useState(courses[0]?.id || null);
  useEffect(() => {
    if (!courses.some((course) => course.id === selectedCourseId)) {
      setSelectedCourseId(courses[0]?.id || null);
    }
  }, [courses, selectedCourseId]);
  const selected = courses.find((course) => course.id === selectedCourseId);
  const lessons = videos.filter((video) => video.course_id === selectedCourseId);

  return (
    <div className="page-stack">
      <section className="page-heading compact">
        <div><h1>Courses</h1><p>Organize lessons and grow each course question bank.</p></div>
        <button className="button primary" onClick={createCourse}><Plus size={17} /> New course</button>
      </section>
      <div className="master-detail">
        <section className="master-list">
          <div className="list-label">All courses <span>{courses.length}</span></div>
          {courses.map((course) => (
            <button
              key={course.id}
              className={classNames("master-item", selectedCourseId === course.id && "active")}
              onClick={() => setSelectedCourseId(course.id)}
            >
              <div className="course-letter small">{course.title.slice(0, 1).toUpperCase()}</div>
              <div>
                <strong>{course.title}</strong>
                <span>{course.video_count} lessons · {course.question_count} questions</span>
              </div>
              <ChevronRight size={16} />
            </button>
          ))}
          {!courses.length && <EmptyState icon={GraduationCap} title="No courses" body="Create your first course." />}
        </section>

        <section className="detail-pane">
          {selected ? (
            <>
              <div className="detail-title">
                <div>
                  <p className="eyebrow">Course</p>
                  <h2>{selected.title}</h2>
                  <p>{selected.description || "No course description yet."}</p>
                </div>
                <div className="detail-title-actions">
                  <div className="detail-metrics">
                    <span><strong>{selected.video_count}</strong> lessons</span>
                    <span><strong>{selected.question_count}</strong> questions</span>
                  </div>
                  <button className="icon-button danger" onClick={() => deleteCourse(selected)} title="Delete course">
                    <Trash2 size={17} />
                  </button>
                </div>
              </div>
              <div className="section-header inset">
                <div><h3>Lessons</h3><p>Videos currently assigned to this course.</p></div>
              </div>
              <div className="content-list">
                {lessons.map((video) => (
                  <div key={video.id} className="content-row">
                    <button className="content-row-main" onClick={() => openStudyRoom(video.id)}>
                      <div className="lesson-symbol"><Video size={17} /></div>
                      <div><strong>{video.title}</strong><span>{formatDuration(video.duration_s)} · {video.question_count} questions</span></div>
                      <StatusPill status={video.status} />
                      <ChevronRight size={17} />
                    </button>
                    <button className="icon-button danger row-delete" onClick={() => deleteLesson(video)} title="Delete lesson">
                      <Trash2 size={16} />
                    </button>
                  </div>
                ))}
                {!lessons.length && <EmptyState icon={ListVideo} title="No lessons in this course" body="Use Add lesson to upload one." />}
              </div>
            </>
          ) : <EmptyState icon={GraduationCap} title="Choose a course" body="Select a course to inspect its lessons." />}
        </section>
      </div>
    </div>
  );
}

function LibraryView({ videos, courses, openStudyRoom, openUpload, refresh, deleteLesson }) {
  const [query, setQuery] = useState("");
  const [courseId, setCourseId] = useState("");
  const [status, setStatus] = useState("");
  const filtered = videos.filter((video) => {
    const matchesQuery = video.title.toLowerCase().includes(query.toLowerCase());
    return matchesQuery && (!courseId || video.course_id === courseId) && (!status || video.status === status);
  });

  return (
    <div className="page-stack">
      <section className="page-heading compact">
        <div><h1>Lesson library</h1><p>Browse uploaded media and processing state.</p></div>
        <button className="button primary" onClick={openUpload}><Upload size={17} /> Add lesson</button>
      </section>
      <section className="surface">
        <div className="filter-bar">
          <label className="search-field"><Search size={17} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search lessons" /></label>
          <select value={courseId} onChange={(event) => setCourseId(event.target.value)}>
            <option value="">All courses</option>
            {courses.map((course) => <option key={course.id} value={course.id}>{course.title}</option>)}
          </select>
          <select value={status} onChange={(event) => setStatus(event.target.value)}>
            <option value="">All statuses</option>
            <option value="ready">Ready</option>
            <option value="pending">Pending</option>
            <option value="transcribing">Transcribing</option>
            <option value="failed">Failed</option>
          </select>
          <button className="icon-button" onClick={refresh} title="Refresh library"><RefreshCw size={17} /></button>
        </div>
        <div className="library-grid">
          {filtered.map((video) => (
            <article className="lesson-tile" key={video.id}>
              <button className="lesson-tile-open" onClick={() => openStudyRoom(video.id)}>
                <div className="lesson-preview">
                  <div className="preview-course">{video.course_title}</div>
                  <div className="preview-play"><Play size={20} fill="currentColor" /></div>
                  <span>{formatDuration(video.duration_s)}</span>
                </div>
              </button>
              <div className="tile-body">
                <div><h3>{video.title}</h3><p>{formatDate(video.created_at)}</p></div>
                <button className="icon-button danger tile-delete" onClick={() => deleteLesson(video)} title="Delete lesson">
                  <Trash2 size={16} />
                </button>
              </div>
              <div className="tile-footer">
                <StatusPill status={video.status} />
                <span>{video.topic_count} topics</span>
                <span>{video.question_count} questions</span>
              </div>
            </article>
          ))}
          {!filtered.length && <EmptyState icon={FolderOpen} title="No matching lessons" body="Change the filters or upload new media." />}
        </div>
      </section>
    </div>
  );
}

function QuestionBankView({ courses, videos, notify }) {
  const [courseId, setCourseId] = useState(courses[0]?.id || "");
  const [count, setCount] = useState(10);
  const [type, setType] = useState("");
  const [questions, setQuestions] = useState([]);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!courseId && courses.length) setCourseId(courses[0].id);
  }, [courses]);

  const generate = async () => {
    if (!courseId) return;
    setLoading(true);
    try {
      const params = new URLSearchParams({ count: String(count) });
      if (type) params.set("type", type);
      const data = await api(`/api/courses/${courseId}/questions/random?${params}`);
      setQuestions(data.questions || []);
    } catch (error) {
      notify(error.message, "danger");
    } finally {
      setLoading(false);
    }
  };

  const course = courses.find((item) => item.id === courseId);
  const sourceCount = new Set(questions.map((question) => question.source_video.id)).size;

  return (
    <div className="page-stack">
      <section className="page-heading compact">
        <div><h1>Question bank</h1><p>Sample questions across every lesson in a course.</p></div>
      </section>
      <section className="bank-toolbar">
        <label><span>Course</span><select value={courseId} onChange={(event) => setCourseId(event.target.value)}>
          <option value="">Select a course</option>
          {courses.map((item) => <option key={item.id} value={item.id}>{item.title}</option>)}
        </select></label>
        <label><span>Questions</span><input type="number" min="1" max="100" value={count} onChange={(event) => setCount(Number(event.target.value))} /></label>
        <label><span>Type</span><select value={type} onChange={(event) => setType(event.target.value)}>
          <option value="">All types</option><option value="mcq">Multiple choice</option><option value="true_false">True / false</option><option value="essay">Essay</option>
        </select></label>
        <button className="button primary" onClick={generate} disabled={!courseId || loading}>
          {loading ? <LoaderCircle className="spin" size={17} /> : <Sparkles size={17} />} Generate set
        </button>
      </section>

      {questions.length > 0 && (
        <div className="bank-summary">
          <div><strong>{questions.length}</strong><span>questions</span></div>
          <div><strong>{sourceCount}</strong><span>source lessons</span></div>
          <div><strong>{course?.title}</strong><span>course</span></div>
        </div>
      )}

      <section className="question-bank-list">
        {questions.map((question, index) => (
          <article className="bank-question" key={question.id}>
            <div className="question-index">{String(index + 1).padStart(2, "0")}</div>
            <div className="question-content">
              <div className="question-meta">
                <span>{question.source_video.title}</span>
                {question.topic_label && <span>{question.topic_label}</span>}
                <span>{titleCase(question.question_type)}</span>
              </div>
              <h3>{question.stem}</h3>
              {question.choices?.length > 0 && (
                <div className="choice-preview">
                  {question.choices.map((choice) => <span key={choice.label}><b>{choice.label}</b>{choice.text}</span>)}
                </div>
              )}
            </div>
          </article>
        ))}
        {!questions.length && <EmptyState icon={CircleHelp} title="Build a question set" body="Choose a course, count, and optional type to draw from its lesson bank." />}
      </section>
    </div>
  );
}

function StudyRoom({ videos, selectedVideoId, setSelectedVideoId, refreshVideos, notify, userId, studentName }) {
  const [detail, setDetail] = useState(null);
  const [transcript, setTranscript] = useState(null);
  const [tab, setTab] = useState("lesson");
  const playerRef = useRef(null);

  const load = async () => {
    if (!selectedVideoId) return;
    try {
      const [detailData, transcriptData] = await Promise.all([
        api(`/api/videos/${selectedVideoId}`),
        api(`/api/videos/${selectedVideoId}/transcript`)
      ]);
      setDetail(detailData);
      setTranscript(transcriptData);
    } catch (error) {
      notify(error.message, "danger");
    }
  };

  useEffect(() => {
    setDetail(null);
    setTranscript(null);
    load();
  }, [selectedVideoId]);

  useEffect(() => {
    if (!selectedVideoId || !detail || TERMINAL_VIDEO_STATUSES.has(detail.video.status)) return undefined;
    const stream = new EventSource(`${API_BASE}/api/videos/${selectedVideoId}/events`);
    stream.addEventListener("video", (event) => {
      const snapshot = JSON.parse(event.data);
      setDetail(snapshot);
      refreshVideos();
      if (TERMINAL_VIDEO_STATUSES.has(snapshot.video.status)) {
        stream.close();
        load();
      }
    });
    stream.onerror = () => stream.close();
    return () => stream.close();
  }, [selectedVideoId, detail?.video?.status]);

  const seek = (seconds) => {
    if (!playerRef.current) return;
    playerRef.current.currentTime = Number(seconds || 0);
    playerRef.current.play().catch(() => {});
  };

  if (!videos.length) {
    return <EmptyState icon={Film} title="No lessons available" body="Upload a lesson before entering the study room." />;
  }

  return (
    <div className="study-layout">
      <aside className="study-rail">
        <div className="list-label">Lessons <span>{videos.length}</span></div>
        {videos.map((video) => (
          <button key={video.id} className={classNames("study-lesson", selectedVideoId === video.id && "active")} onClick={() => setSelectedVideoId(video.id)}>
            <div className="lesson-symbol"><Play size={15} /></div>
            <div><strong>{video.title}</strong><span>{video.course_title}</span></div>
            <StatusDot status={video.status} />
          </button>
        ))}
      </aside>

      <section className="study-main">
        {!detail ? <LoadingState label="Loading lesson" /> : (
          <>
            <div className="study-header">
              <div>
                <div className="breadcrumb"><span>{detail.video.course_title}</span><ChevronRight size={13} /><span>Lesson</span></div>
                <h1>{detail.video.title}</h1>
              </div>
              <StatusPill status={detail.video.status} />
            </div>

            <div className="study-tabs">
              {[
                ["lesson", "Lesson", Film],
                ["transcript", "Transcript", ListVideo],
                ["chat", "Chat", MessageSquare],
                ["assessment", "Assessment", FileQuestion]
              ].map(([id, label, Icon]) => (
                <button key={id} className={tab === id ? "active" : ""} onClick={() => setTab(id)}><Icon size={16} />{label}</button>
              ))}
            </div>

            {tab === "lesson" && (
              <LessonTab detail={detail} videoId={selectedVideoId} playerRef={playerRef} seek={seek} transcript={transcript} notify={notify} refreshVideos={refreshVideos} />
            )}
            {tab === "transcript" && <TranscriptTab transcript={transcript} seek={seek} />}
            {tab === "chat" && <ChatTab videoId={selectedVideoId} playerRef={playerRef} notify={notify} userId={userId} studentName={studentName} />}
            {tab === "assessment" && <AssessmentTab videoId={selectedVideoId} notify={notify} userId={userId} />}
          </>
        )}
      </section>
    </div>
  );
}

function LessonTab({ detail, videoId, playerRef, seek, transcript, notify, refreshVideos }) {
  const remove = async () => {
    if (!window.confirm(`Delete "${detail.video.title}" and all generated data?`)) return;
    try {
      await api(`/api/videos/${videoId}`, { method: "DELETE" });
      notify("Lesson deleted.", "success");
      refreshVideos();
    } catch (error) {
      notify(error.message, "danger");
    }
  };

  return (
    <div className="lesson-tab">
      <div className="media-column">
        <video ref={playerRef} className="main-player" controls preload="metadata" src={`${API_BASE}/api/videos/${videoId}/media`}>
          <track src={`${API_BASE}/api/videos/${videoId}/transcript.vtt`} kind="subtitles" srcLang="en" label="Transcript" default />
        </video>
        <div className="media-meta">
          <div>
            <span>{formatDuration(detail.video.duration_s)}</span>
            <span>{detail.video.topic_count} topics</span>
            <span>{detail.video.question_count} questions</span>
          </div>
          <button className="icon-button danger" onClick={remove} title="Delete lesson"><Trash2 size={17} /></button>
        </div>
        <section className="content-section">
          <h2>Summary</h2>
          <p className="reading-copy">{detail.summary || "The summary will appear when processing completes."}</p>
        </section>
        <section className="content-section">
          <h2>Topics</h2>
          <div className="topic-timeline">
            {(detail.topics || []).map((topic) => (
              <button key={topic.id} onClick={() => seek(topic.start_s)}>
                <span>{formatDuration(topic.start_s)}</span>
                <strong>{topic.label}</strong>
                <Play size={14} />
              </button>
            ))}
            {!detail.topics?.length && <p className="muted-copy">Topics are not ready yet.</p>}
          </div>
        </section>
      </div>
      <aside className="lesson-side">
        <h3>Transcript preview</h3>
        <div className="mini-transcript">
          {(transcript?.segments || []).slice(0, 12).map((segment) => (
            <button key={segment.seq_index} onClick={() => seek(segment.start_s)}>
              <span>{formatDuration(segment.start_s)}</span>
              <p>{segment.text}</p>
            </button>
          ))}
          {!transcript?.segments?.length && <p className="muted-copy">Transcript is not ready yet.</p>}
        </div>
      </aside>
    </div>
  );
}

function TranscriptTab({ transcript, seek }) {
  const [query, setQuery] = useState("");
  const segments = (transcript?.segments || []).filter((segment) => segment.text.toLowerCase().includes(query.toLowerCase()));
  return (
    <section className="transcript-pane">
      <div className="filter-bar">
        <label className="search-field"><Search size={17} /><input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="Search transcript" /></label>
        <span>{segments.length} segments</span>
      </div>
      <div className="full-transcript">
        {segments.map((segment) => (
          <button key={segment.seq_index} onClick={() => seek(segment.start_s)}>
            <span className="timestamp">{formatDuration(segment.start_s)}</span>
            <p>{segment.text}</p>
            <Play size={14} />
          </button>
        ))}
        {!segments.length && <EmptyState icon={ListVideo} title="No transcript segments" body="Processing may still be underway." />}
      </div>
    </section>
  );
}

function ChatTab({ videoId, playerRef, notify, userId, studentName }) {
  const [chats, setChats] = useState([]);
  const [activeId, setActiveId] = useState(null);
  const [active, setActive] = useState(null);
  const [name, setName] = useState("");
  const [message, setMessage] = useState("");

  const loadChats = async (preferredId) => {
    try {
      const data = await api(`/api/users/${userId}/chats?video_id=${videoId}`);
      setChats(data.chats || []);
      const next = preferredId || activeId || data.chats?.[0]?.conversation_id;
      if (next) await loadChat(next);
      else { setActiveId(null); setActive(null); }
    } catch (error) {
      notify(error.message, "danger");
    }
  };

  const loadChat = async (conversationId) => {
    const data = await api(`/api/users/${userId}/chats/${conversationId}`);
    setActiveId(conversationId);
    setActive(data);
  };

  useEffect(() => {
    setActiveId(null);
    setActive(null);
    loadChats();
  }, [videoId, userId]);

  useEffect(() => {
    if (!activeId || !active?.is_waiting) return undefined;
    const stream = new EventSource(`${API_BASE}/api/users/${userId}/chats/${activeId}/events`);
    stream.addEventListener("chat", (event) => {
      const snapshot = JSON.parse(event.data);
      setActive(snapshot);
      if (!snapshot.is_waiting) {
        stream.close();
        loadChats(activeId);
      }
    });
    stream.onerror = () => stream.close();
    return () => stream.close();
  }, [activeId, active?.is_waiting]);

  const create = async () => {
    const data = await api(`/api/videos/${videoId}/chats`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ user_id: userId, name: name.trim() || `Study chat ${chats.length + 1}` })
    });
    setName("");
    await loadChats(data.conversation_id);
  };

  const send = async (event) => {
    event.preventDefault();
    if (!activeId || !message.trim() || active?.is_waiting) return;
    const value = message.trim();
    setMessage("");
    setActive((current) => ({
      ...current,
      is_waiting: true,
      messages: [...(current?.messages || []), { id: crypto.randomUUID(), role: "user", content: value, sources: [], cached: false, cache_similarity: null }]
    }));
    try {
      const response = await api(`/api/chats/${activeId}/messages`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ user_id: userId, message: value, history: [] })
      });
      if (response.cached) {
        notify(`Semantic cache hit (${Math.round(response.cache_similarity * 100)}% similar).`, "success");
      }
      await loadChat(activeId);
    } catch (error) {
      notify(error.message, "danger");
      await loadChat(activeId);
    }
  };

  const remove = async () => {
    if (!activeId || !window.confirm(`Delete "${active?.name}"?`)) return;
    await api(`/api/users/${userId}/chats/${activeId}`, { method: "DELETE" });
    await loadChats();
  };

  const seek = (seconds) => {
    if (!playerRef.current) return;
    playerRef.current.currentTime = seconds;
    playerRef.current.play().catch(() => {});
  };

  return (
    <div className="chat-layout">
      <aside className="chat-list">
        <div className="chat-create">
          <input value={name} onChange={(event) => setName(event.target.value)} placeholder="New chat name" />
          <button className="icon-button primary-icon" onClick={create} title="Start chat"><Plus size={17} /></button>
        </div>
        {chats.map((chat) => (
          <button key={chat.conversation_id} className={classNames("chat-list-item", activeId === chat.conversation_id && "active")} onClick={() => loadChat(chat.conversation_id)}>
            <MessageSquare size={16} />
            <div><strong>{chat.name}</strong><span>{chat.message_count} messages</span></div>
            {chat.is_waiting && <LoaderCircle className="spin" size={14} />}
          </button>
        ))}
        {!chats.length && <p className="muted-copy padded">Start a chat for this lesson.</p>}
      </aside>
      <section className="chat-conversation">
        {active ? (
          <>
            <div className="chat-header">
              <div><h2>{active.name}</h2><span>{studentName} · {active.video_title}</span></div>
              <button className="icon-button danger" onClick={remove} title="Delete chat"><Trash2 size={17} /></button>
            </div>
            <div className="message-feed">
              {(active.messages || []).map((item) => (
                <div key={item.id} className={classNames("message", item.role)}>
                  <div className="message-avatar">{item.role === "assistant" ? <Bot size={16} /> : <UserRound size={16} />}</div>
                  <div>
                    <div className="message-meta">
                      <span>{item.role === "assistant" ? "Nexa" : "You"}</span>
                      {item.role === "assistant" && (
                        <span
                          className={classNames("response-origin", item.cached ? "cache" : "llm")}
                          title={item.cached ? `Semantic cache match: ${Math.round((item.cache_similarity || 0) * 100)}%` : "Generated by Gemma"}
                        >
                          {item.cached
                            ? `Semantic cache · ${Math.round((item.cache_similarity || 0) * 100)}%`
                            : "Gemma · live response"}
                        </span>
                      )}
                    </div>
                    <p>{item.content}</p>
                    {!!item.sources?.length && (
                      <div className="message-sources">
                        {item.sources.slice(0, 4).map((source) => (
                          <button key={source.seq_index} onClick={() => seek(source.start_s)}>
                            <Clock3 size={13} /> {formatDuration(source.start_s)}
                          </button>
                        ))}
                      </div>
                    )}
                  </div>
                </div>
              ))}
              {active.is_waiting && <div className="thinking"><LoaderCircle className="spin" size={16} /> Nexa is thinking</div>}
            </div>
            <form className="message-composer" onSubmit={send}>
              <textarea value={message} onChange={(event) => setMessage(event.target.value)} placeholder="Ask about the lesson or anything related..." disabled={active.is_waiting} />
              <button className="icon-button primary-icon" type="submit" disabled={!message.trim() || active.is_waiting} aria-label="Send message"><Send size={18} /></button>
            </form>
          </>
        ) : <EmptyState icon={MessageSquare} title="Select or start a chat" body="Your conversation history stays attached to this lesson." />}
      </section>
    </div>
  );
}

function AssessmentTab({ videoId, notify, userId }) {
  const [groups, setGroups] = useState([]);
  const [attemptId, setAttemptId] = useState(null);
  const [answers, setAnswers] = useState({});
  const [result, setResult] = useState(null);
  const [loading, setLoading] = useState(false);
  const [justifications, setJustifications] = useState({});

  const questions = groups.flatMap((group) => group.questions.map((question) => ({ ...question, topicLabel: group.label })));

  const loadQuestions = async () => {
    setLoading(true);
    try {
      const data = await api(`/api/videos/${videoId}/questions`);
      setGroups(data.topics || []);
      setAttemptId(null);
      setResult(null);
      setAnswers({});
    } catch (error) {
      notify(error.message, "danger");
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => { loadQuestions(); }, [videoId, userId]);

  const start = async () => {
    const data = await api(`/api/videos/${videoId}/exams/start`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ user_id: userId })
    });
    setAttemptId(data.attempt_id);
    notify("Attempt started.", "success");
  };

  const submit = async () => {
    const missing = questions.find((question) => !String(answers[question.id] || "").trim());
    if (missing) {
      notify("Answer every question before submitting.", "danger");
      return;
    }
    const data = await api(`/api/exams/${attemptId}/submit`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        answers: questions.map((question) => ({ question_id: question.id, user_answer: answers[question.id] }))
      })
    });
    setResult(data);
  };

  useEffect(() => {
    if (!attemptId || !result?.is_waiting) return undefined;
    const stream = new EventSource(`${API_BASE}/api/exams/${attemptId}/events`);
    stream.addEventListener("exam", (event) => {
      const snapshot = JSON.parse(event.data);
      setResult(snapshot);
      if (!snapshot.is_waiting) stream.close();
    });
    stream.onerror = () => stream.close();
    return () => stream.close();
  }, [attemptId, result?.is_waiting]);

  const justify = async (answerId) => {
    setJustifications((current) => ({ ...current, [answerId]: { is_waiting: true } }));
    try {
      const initial = await api(`/api/exams/${attemptId}/answers/${answerId}/justification/start`, { method: "POST" });
      if (!initial.is_waiting) {
        setJustifications((current) => ({ ...current, [answerId]: initial }));
        return;
      }
      const stream = new EventSource(`${API_BASE}/api/exams/${attemptId}/answers/${answerId}/justification/events`);
      stream.addEventListener("justification", (event) => {
        const snapshot = JSON.parse(event.data);
        setJustifications((current) => ({ ...current, [answerId]: snapshot }));
        if (!snapshot.is_waiting) stream.close();
      });
      stream.onerror = () => stream.close();
    } catch (error) {
      notify(error.message, "danger");
    }
  };

  return (
    <section className="assessment-pane">
      <div className="assessment-toolbar">
        <div><h2>Lesson assessment</h2><p>{questions.length} generated questions across {groups.length} topics.</p></div>
        {!attemptId ? (
          <button className="button primary" onClick={start} disabled={!questions.length}><Play size={17} /> Start attempt</button>
        ) : !result ? (
          <button className="button primary" onClick={submit}><Check size={17} /> Submit answers</button>
        ) : result.is_waiting ? (
          <span className="waiting-label"><LoaderCircle className="spin" size={16} /> Grading {result.pending_count} answers</span>
        ) : (
          <div className="score-badge"><strong>{result.total_score}</strong><span>total score</span></div>
        )}
      </div>
      {loading ? <LoadingState label="Loading questions" /> : (
        <div className="exam-list">
          {questions.map((question, index) => {
            const graded = result?.answers?.find((answer) => answer.question_id === question.id) || result?.breakdown?.find((answer) => answer.question_id === question.id);
            const justification = graded?.answer_id ? justifications[graded.answer_id] : null;
            return (
              <article className={classNames("exam-question", graded && (graded.is_correct ? "correct" : "incorrect"))} key={question.id}>
                <div className="exam-number">{index + 1}</div>
                <div className="exam-body">
                  <div className="question-meta"><span>{question.topicLabel}</span><span>{titleCase(question.question_type)}</span></div>
                  <h3>{question.stem}</h3>
                  {question.choices?.length ? (
                    <div className="answer-options">
                      {question.choices.map((choice) => (
                        <label key={choice.label} className={answers[question.id] === choice.label ? "selected" : ""}>
                          <input type="radio" name={question.id} value={choice.label} checked={answers[question.id] === choice.label} onChange={() => setAnswers((current) => ({ ...current, [question.id]: choice.label }))} disabled={!!result} />
                          <b>{choice.label}</b><span>{choice.text}</span>
                        </label>
                      ))}
                    </div>
                  ) : (
                    <textarea value={answers[question.id] || ""} onChange={(event) => setAnswers((current) => ({ ...current, [question.id]: event.target.value }))} placeholder="Write your answer" disabled={!!result} />
                  )}
                  {graded && (
                    <div className="grade-feedback">
                      <span className={graded.is_correct ? "good-text" : "bad-text"}>{graded.is_correct ? "Correct" : "Needs review"} · {graded.score}/100</span>
                      <button className="text-button" onClick={() => justify(graded.answer_id)} disabled={justification?.is_waiting}>
                        {justification?.is_waiting ? <LoaderCircle className="spin" size={14} /> : <Sparkles size={14} />} Explain
                      </button>
                      {justification?.justification && <p>{justification.justification}</p>}
                    </div>
                  )}
                </div>
              </article>
            );
          })}
          {!questions.length && <EmptyState icon={FileQuestion} title="No generated questions" body="Questions appear when lesson processing finishes." />}
        </div>
      )}
    </section>
  );
}

function UploadDialog({ courses, close, complete, notify }) {
  const [mode, setMode] = useState("file");
  const [busy, setBusy] = useState(false);
  const [title, setTitle] = useState("");
  const [courseId, setCourseId] = useState(courses[0]?.id || "");
  const [file, setFile] = useState(null);
  const [downloadUrl, setDownloadUrl] = useState("");
  const [fileName, setFileName] = useState("");

  const submit = async (event) => {
    event.preventDefault();
    if (!courseId) {
      notify("Create or select a course first.", "danger");
      return;
    }
    setBusy(true);
    try {
      let data;
      if (mode === "file") {
        if (!file) throw new Error("Choose a video or audio file.");
        const form = new FormData();
        form.append("title", title.trim() || file.name);
        form.append("course_id", courseId);
        form.append("file", file);
        data = await api("/api/videos/upload", { method: "POST", body: form });
      } else {
        data = await api("/api/mux/import-download-url", {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({
            title: title.trim(),
            course_id: courseId,
            download_url: downloadUrl.trim(),
            upload_url: null,
            file_name: fileName.trim() || null
          })
        });
      }
      notify("Lesson queued for processing.", "success");
      complete(data.video_id);
    } catch (error) {
      notify(error.message, "danger");
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal title="Add lesson" close={close}>
      <div className="segmented">
        <button className={mode === "file" ? "active" : ""} onClick={() => setMode("file")}><Upload size={16} /> Local file</button>
        <button className={mode === "mux" ? "active" : ""} onClick={() => setMode("mux")}><CloudDownload size={16} /> Mux URL</button>
      </div>
      <form className="form-stack" onSubmit={submit}>
        <label><span>Course</span><select value={courseId} onChange={(event) => setCourseId(event.target.value)} required>
          <option value="">Select course</option>
          {courses.map((course) => <option key={course.id} value={course.id}>{course.title}</option>)}
        </select></label>
        <label><span>Lesson title</span><input value={title} onChange={(event) => setTitle(event.target.value)} placeholder="Introduction to thermodynamics" required={mode === "mux"} /></label>
        {mode === "file" ? (
          <label className="file-drop">
            <input type="file" accept="video/*,audio/*" onChange={(event) => setFile(event.target.files?.[0] || null)} />
            <Upload size={24} />
            <strong>{file ? file.name : "Choose media file"}</strong>
            <span>{file ? `${(file.size / 1024 / 1024).toFixed(1)} MB` : "Video or audio, up to 1 GiB"}</span>
          </label>
        ) : (
          <>
            <label><span>Download URL</span><input type="url" value={downloadUrl} onChange={(event) => setDownloadUrl(event.target.value)} placeholder="https://..." required /></label>
            <label><span>File name <small>optional</small></span><input value={fileName} onChange={(event) => setFileName(event.target.value)} placeholder="lesson.mp4" /></label>
          </>
        )}
        <div className="modal-actions">
          <button className="button secondary" type="button" onClick={close}>Cancel</button>
          <button className="button primary" type="submit" disabled={busy || !courseId}>
            {busy ? <LoaderCircle className="spin" size={17} /> : mode === "file" ? <Upload size={17} /> : <CloudDownload size={17} />}
            {busy ? "Queuing" : "Add lesson"}
          </button>
        </div>
      </form>
    </Modal>
  );
}

function CourseDialog({ close, complete, notify }) {
  const [title, setTitle] = useState("");
  const [description, setDescription] = useState("");
  const [busy, setBusy] = useState(false);
  const submit = async (event) => {
    event.preventDefault();
    setBusy(true);
    try {
      await api("/api/courses", {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ title: title.trim(), description: description.trim() || null })
      });
      notify("Course created.", "success");
      complete();
    } catch (error) {
      notify(error.message, "danger");
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title="Create course" close={close}>
      <form className="form-stack" onSubmit={submit}>
        <label><span>Course title</span><input value={title} onChange={(event) => setTitle(event.target.value)} placeholder="Computer Science 101" required autoFocus /></label>
        <label><span>Description <small>optional</small></span><textarea value={description} onChange={(event) => setDescription(event.target.value)} placeholder="What learners will cover" /></label>
        <div className="modal-actions">
          <button className="button secondary" type="button" onClick={close}>Cancel</button>
          <button className="button primary" type="submit" disabled={busy || !title.trim()}>{busy ? <LoaderCircle className="spin" size={17} /> : <Plus size={17} />} Create course</button>
        </div>
      </form>
    </Modal>
  );
}

function Modal({ title, close, children }) {
  useEffect(() => {
    const handler = (event) => event.key === "Escape" && close();
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [close]);
  return (
    <div className="modal-backdrop" onMouseDown={(event) => event.target === event.currentTarget && close()}>
      <section className="modal" role="dialog" aria-modal="true">
        <header><h2>{title}</h2><button className="icon-button" onClick={close} aria-label="Close"><X size={18} /></button></header>
        {children}
      </section>
    </div>
  );
}

function StatusPill({ status }) {
  return <span className={classNames("status-pill", status)}>{status === "ready" && <Check size={12} />}{titleCase(status)}</span>;
}

function StatusDot({ status }) {
  return <span className={classNames("status-dot", status)} title={titleCase(status)} />;
}

function LoadingState({ label }) {
  return <div className="loading-state"><LoaderCircle className="spin" size={22} /><span>{label}</span></div>;
}

function EmptyState({ icon: Icon, title, body }) {
  return <div className="empty-state"><Icon size={24} /><strong>{title}</strong><span>{body}</span></div>;
}

createRoot(document.getElementById("root")).render(<App />);
