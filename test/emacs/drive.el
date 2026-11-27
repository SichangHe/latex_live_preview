;;; drive.el --- Drives ltxp.el in batch Emacs for test/emacs/run.mjs -*- lexical-binding: t -*-
;; Env: LTXP_PLUGIN (ltxp.el), LTXP_TEST_DIR (work dir with chap.tex), LTXP_TEST_CMD (server
;; command, space-separated).  Reads commands from `cmd' in the work dir and writes `status'
;; (`ready' once Eglot connected) and `point' (0-based line and column) there.
(load (getenv "LTXP_PLUGIN"))
(setq browse-url-browser-function (lambda (url &rest _) (drive-put "url" url)))
(setq ltxp-command (split-string (getenv "LTXP_TEST_CMD")))
(setq eglot-send-changes-idle-time 0.05)
(ltxp-setup)
(defvar dir (file-name-as-directory (getenv "LTXP_TEST_DIR")))
(defun drive-put (name text) (with-temp-file (concat dir name) (insert text)))
(find-file (concat dir "chap.tex"))
(sleep-for 1)
(when (eglot-current-server) (drive-put "status" "auto-started") (kill-emacs 1))
(ltxp-preview)
(run-hooks 'post-command-hook)
(defvar n-done 0)
(while t
  (accept-process-output nil 0.05)
  (when (eglot-current-server) (drive-put "status" "ready"))
  (let ((cmds (when (file-exists-p (concat dir "cmd"))
                (with-temp-buffer (insert-file-contents (concat dir "cmd")) (split-string (buffer-string) "\n" t)))))
    (dolist (c (nthcdr n-done cmds))
      (setq n-done (1+ n-done))
      (pcase (split-string c " ")
        (`("insert" ,l . ,words)
         (goto-char (point-min)) (forward-line (string-to-number l))
         (insert (concat (string-join words " ") " "))
         ;; Eglot sends changes from an idle timer, and idle timers never fire in batch mode.
         (eglot--signal-textDocument/didChange))
        (`("move" ,l ,col)
         (goto-char (point-min)) (forward-line (string-to-number l)) (forward-char (string-to-number col))
         (run-hooks 'post-command-hook))
        (`("quit")
         ;; Docker kills every process once Emacs exits, so let the server clean up first.
         (eglot-shutdown (eglot-current-server))
         (sleep-for 1)
         (kill-emacs 0)))))
  (drive-put "point" (format "%d %d" (1- (line-number-at-pos)) (current-column))))
