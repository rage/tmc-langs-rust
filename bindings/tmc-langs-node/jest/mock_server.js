const express = require("express");
const { readFileSync } = require("fs");
const app = express();
process.chdir(__dirname);
app.get("/api/v8/core/exercises/details", (_req, res) => {
  res.send({
    exercises: [
      {
        id: 1,
        course_name: "some course",
        exercise_name: "on disk exercise with update and submission",
        checksum: "new checksum",
      },
      {
        id: 2,
        course_name: "some course",
        exercise_name: "on disk exercise without update",
        checksum: "new checksum",
      },
      {
        id: 3,
        course_name: "another course",
        exercise_name: "not on disk exercise with submission",
        checksum: "new checksum",
      },
      {
        id: 4,
        course_name: "another course",
        exercise_name: "not on disk exercise without submission",
        checksum: "new checksum",
      },
    ],
  });
});
app.get("/api/v8/exercises/:x/users/current/submissions", (_req, res) => {
  res.send([
    {
      id: 1,
      user_id: 1,
      pretest_error: null,
      created_at: "2021-03-24T13:28:59.103+02:00",
      exercise_name: "Exercise",
      course_id: 1,
      processed: true,
      all_tests_passed: false,
      points: null,
      processing_tried_at: "2021-03-24T13:28:59.170+02:00",
      processing_began_at: "2021-03-24T13:28:59.356+02:00",
      processing_completed_at: "2021-03-24T13:29:23.500+02:00",
      times_sent_to_sandbox: 1,
      processing_attempts_started_at: "2021-03-24T13:28:59.103+02:00",
      params_json: "{}",
      requires_review: false,
      requests_review: false,
      reviewed: false,
      message_for_reviewer: "",
      newer_submission_reviewed: false,
      review_dismissed: false,
      paste_available: false,
      message_for_paste: "",
      paste_key: null,
    },
  ]);
});
app.get("/api/v8/core/courses/:x", (_req, res) => {
  res.send({
    course: {
      id: 588,
      name: "mooc-2020-ohjelmointi",
      title: "Ohjelmoinnin MOOC 2020, Ohjelmoinnin perusteet",
      description:
        "Aikataulutettu Ohjelmoinnin MOOC 2020. Kurssin ensimmäinen puolisko. Tästä kurssista voi hakea opinto-oikeutta Helsingin yliopiston tietojenkäsittelytieteen osastolle.",
      details_url: "https://tmc.mooc.fi/api/v8/core/courses/588",
      unlock_url: "https://tmc.mooc.fi/api/v8/core/courses/588/unlock",
      reviews_url: "https://tmc.mooc.fi/api/v8/core/courses/588/reviews",
      comet_url: "https://tmc.mooc.fi:8443/comet",
      spyware_urls: ["http://snapshots01.mooc.fi/"],
      unlockables: [],
      exercises: [
        {
          id: 12,
          name: "unchanged",
          locked: false,
          deadline_description: "2020-01-20 23:59:59 +0200",
          deadline: "2020-01-20T23:59:59.999+02:00",
          soft_deadline: null,
          soft_deadline_description: null,
          checksum: "ab",
          return_url:
            "https://tmc.mooc.fi/api/v8/core/exercises/81842/submissions",
          zip_url: "https://tmc.mooc.fi/api/v8/core/exercises/81842/download",
          returnable: true,
          requires_review: false,
          attempted: false,
          completed: false,
          reviewed: false,
          all_review_points_given: true,
          memory_limit: null,
          runtime_params: [],
          valgrind_strategy: "fail",
          code_review_requests_enabled: false,
          run_tests_locally_action_enabled: true,
        },
        {
          id: 23,
          name: "updated",
          locked: false,
          deadline_description: "2020-01-20 23:59:59 +0200",
          deadline: "2020-01-20T23:59:59.999+02:00",
          soft_deadline: null,
          soft_deadline_description: null,
          checksum: "zz",
          return_url:
            "https://tmc.mooc.fi/api/v8/core/exercises/81842/submissions",
          zip_url: "https://tmc.mooc.fi/api/v8/core/exercises/81842/download",
          returnable: true,
          requires_review: false,
          attempted: false,
          completed: false,
          reviewed: false,
          all_review_points_given: true,
          memory_limit: null,
          runtime_params: [],
          valgrind_strategy: "fail",
          code_review_requests_enabled: false,
          run_tests_locally_action_enabled: true,
        },
        {
          id: 34,
          name: "new",
          locked: false,
          deadline_description: "2020-01-20 23:59:59 +0200",
          deadline: "2020-01-20T23:59:59.999+02:00",
          soft_deadline: null,
          soft_deadline_description: null,
          checksum: "cd",
          return_url:
            "https://tmc.mooc.fi/api/v8/core/exercises/81842/submissions",
          zip_url: "https://tmc.mooc.fi/api/v8/core/exercises/81842/download",
          returnable: true,
          requires_review: false,
          attempted: false,
          completed: false,
          reviewed: false,
          all_review_points_given: true,
          memory_limit: null,
          runtime_params: [],
          valgrind_strategy: "fail",
          code_review_requests_enabled: false,
          run_tests_locally_action_enabled: true,
        },
      ],
    },
  });
});
app.get("/api/v8/courses/:x/exercises", (_req, res) => {
  res.send([
    {
      id: 1,
      available_points: [
        {
          id: 2,
          exercise_id: 3,
          name: "1",
          requires_review: false,
        },
      ],
      awarded_points: [],
      name: "exercise",
      publish_time: null,
      solution_visible_after: null,
      deadline: null,
      soft_deadline: null,
      disabled: false,
      unlocked: true,
    },
  ]);
});
app.get("/api/v8/core/org/:x/courses", (_req, res) => {
  res.send([
    {
      id: 1,
      name: "course",
      title: "Course",
      description: "Nice course",
      details_url: "https://localhost",
      unlock_url: "https://localhost",
      reviews_url: "https://localhost",
      comet_url: "",
      spyware_urls: [],
    },
  ]);
});
app.get("/api/v8/courses/:x", (_req, res) => {
  res.send({
    name: "course",
    hide_after: null,
    hidden: false,
    cache_version: 1,
    spreadsheet_key: null,
    hidden_if_registered_after: null,
    refreshed_at: "2020-10-12T17:25:23.837+03:00",
    locked_exercise_points_visible: true,
    paste_visibility: null,
    formal_name: null,
    certificate_downloadable: false,
    certificate_unlock_spec: null,
    organization_id: 21,
    disabled_status: "enabled",
    title: "Course",
    description: "Nice course",
    material_url: "https://localhost",
    course_template_id: 1,
    hide_submission_results: false,
    external_scoreboard_url: "",
    organization_slug: "org",
  });
});
app.get("/api/v8/core/exercises/:x", (_req, res) => {
  res.send({
    course_name: "Course",
    course_id: 1,
    code_review_requests_enabled: true,
    run_tests_locally_action_enabled: true,
    exercise_name: "Exercise",
    exercise_id: 1,
    unlocked_at: null,
    deadline: "2020-09-11T23:59:59.999+03:00",
    submissions: [],
  });
});
app.get("/api/v8/core/courses/:x/reviews", (_req, res) => {
  res.send([
    {
      submission_id: 1,
      exercise_name: "Exercise",
      id: 1,
      marked_as_read: false,
      reviewer_name: "Reviewer",
      review_body: "Body",
      points: [],
      points_not_awarded: [],
      url: "url",
      update_url: "update",
      created_at: "2020-09-11T23:59:59.999+03:00",
      updated_at: "2020-09-11T23:59:59.999+03:00",
    },
  ]);
});
app.put("/api/v8/core/courses/:x/reviews/:y", (_req, res) => {
  res.send();
});
app.post("/api/v8/core/exercises/:x/submissions", (_req, res) => {
  res.send({
    show_submission_url: "someurl",
    paste_url: "anotherurl",
    submission_url: "third",
  });
});
app.post("/api/v8/core/submissions/:x/feedback", (_req, res) => {
  res.send({
    api_version: 0,
    status: "processing",
  });
});
app.get("/api/v8/core/submission/:x", (_req, res) => {
  res.send({
    api_version: 0,
    user_id: 1,
    login: "",
    course: "",
    exercise_name: "",
    status: "processing",
    points: [],
    submission_url: "",
    submitted_at: "",
    reviewed: false,
    requests_review: false,
    missing_review_points: [],
  });
});
app.get("/api/v8/org.json", (_req, res) => {
  res.send([
    {
      id: 2,
      name: "demo",
      information: "demo org for playing around",
      slug: "demo",
      verified_at: null,
      verified: true,
      disabled: true,
      disabled_reason: "My test",
      created_at: "2015-08-03T17:26:47.750+03:00",
      updated_at: "2016-03-23T23:19:54.532+02:00",
      hidden: false,
      creator_id: null,
      logo_file_name: null,
      logo_content_type: null,
      logo_file_size: null,
      logo_updated_at: null,
      phone: null,
      contact_information: null,
      email: null,
      website: null,
      pinned: false,
      whitelisted_ips: null,
      logo_path: "missing.png",
    },
  ]);
});
app.get("/api/v8/*/download", (_req, res) => {
  let bytes = readFileSync("./python-exercise.zip");
  res.send(bytes);
});
app.listen(3000, "localhost");
